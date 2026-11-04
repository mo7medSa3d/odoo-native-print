//! Tauri commands callable from the configured Odoo origin and the local
//! Control Center (`index.html`).
//! Every command validates the calling webview's origin in Rust first
//! (ADR-0002): Tauri app commands are registered in `AppManifest::commands`
//! (`build.rs`) and also checked explicitly in Rust so origin confusion
//! (CVE-2026-42184) can never bypass security.

use crate::config;
use print_core::diagnostics::{build_bundle_from_queue, bundle_to_json, JobSummary};
use print_core::paper::{match_paper, match_paper_auto, Form, ScalePolicy};
use print_core::printer::{needs_intervention, PrinterInfo, PrinterKind, PrinterStatus};
use print_core::routing::resolve_printer;
use print_core::security::{origin_is_allowed, validate_job_input};
use print_core::worker::Outcome;
use print_core::{
    BackendError, Job, JobOptions, JobState, PrintBackend, Queue, Worker, WorkerConfig,
};
use print_raw::MonoBitmap;
use print_render::RenderMode;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::{Manager, State};

pub struct AppState {
    pub queue: Arc<Mutex<Queue>>,
    pub configured_origin: RwLock<Option<String>>,
    pub config_dir: PathBuf,
    pub locked_by_policy: bool,
    pub auto_reprint: RwLock<bool>,
    pub worker_wake: Arc<(Mutex<bool>, Condvar)>,
    /// Latest environment probe report from the page (JSON string):
    /// the JavaScript feature/storage facts the Odoo frontend depends
    /// on, surfaced in the overview and the diagnostics bundle so a
    /// broken webview environment is a fact, not a guess.
    pub environment: RwLock<Option<String>>,
    /// Short-lived cache of the last printer enumeration. Windows
    /// printer discovery can take seconds (offline network printers),
    /// and the Control Center polls the overview every few seconds;
    /// enumerating on every call made the whole app feel heavy and
    /// blocked the UI thread. 20s is fresh enough for a settings page.
    pub printer_cache: RwLock<Option<PrinterCache>>,
}

/// One cached printer enumeration (see `AppState::printer_cache`).
pub struct PrinterCache {
    pub at: Instant,
    pub printers: Vec<PrinterInfo>,
    pub default: Option<String>,
}

/// Printer enumeration through the 20s cache (see `PrinterCache`).
fn cached_printers(state: &AppState) -> Result<(Vec<PrinterInfo>, Option<String>), String> {
    const TTL: Duration = Duration::from_secs(20);
    if let Ok(guard) = state.printer_cache.read() {
        if let Some(cache) = &*guard {
            if cache.at.elapsed() < TTL {
                return Ok((cache.printers.clone(), cache.default.clone()));
            }
        }
    }
    let printers = print_win::discover_printers().map_err(|e| e.to_string())?;
    let default = print_win::default_printer_name().ok();
    if let Ok(mut guard) = state.printer_cache.write() {
        *guard = Some(PrinterCache {
            at: Instant::now(),
            printers: printers.clone(),
            default: default.clone(),
        });
    }
    Ok((printers, default))
}

/// PDFium availability is a process-lifetime fact; probing it used to
/// load the 7.5 MB DLL on every overview call.
fn pdfium_ready() -> bool {
    static READY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *READY.get_or_init(|| print_render::Pdfium::load().is_ok())
}

impl AppState {
    pub fn notify_worker(&self) {
        let (lock, cvar) = &*self.worker_wake;
        if let Ok(mut pending) = lock.lock() {
            *pending = true;
            cvar.notify_all();
        }
    }
}

/// Custom deserializer that accepts:
/// 1. Standard JSON number array `[37, 80, 68, 70, ...]`
/// 2. Base64 string `"JVBERi0..."`
/// 3. JavaScript `Uint8Array` serialized as an object `{"0": 37, "1": 80, ...}`
fn deserialize_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BytePayload {
        Seq(Vec<u8>),
        Str(String),
        Map(BTreeMap<usize, u8>),
    }

    match BytePayload::deserialize(deserializer)? {
        BytePayload::Seq(v) => Ok(v),
        BytePayload::Str(s) => decode_base64(&s).map_err(serde::de::Error::custom),
        BytePayload::Map(m) => {
            // A sparse index map ({0:.., 2:..}) would silently
            // compress into a shorter, shifted payload. Reject gaps:
            // keys must be exactly 0..len in order (BTreeMap iterates
            // ascending, so position must equal key).
            if m.keys().enumerate().any(|(i, k)| *k != i) {
                return Err(serde::de::Error::custom(
                    "sparse byte map payload: indices must be contiguous from 0",
                ));
            }
            Ok(m.into_values().collect())
        }
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.is_empty() {
        return Ok(Vec::new());
    }
    let val = |b: u8| -> Result<u8, String> {
        match b {
            b'A'..=b'Z' => Ok(b - b'A'),
            b'a'..=b'z' => Ok(b - b'a' + 26),
            b'0'..=b'9' => Ok(b - b'0' + 52),
            b'+' | b'-' => Ok(62),
            b'/' | b'_' => Ok(63),
            _ => Err(format!("invalid base64 byte: {b}")),
        }
    };
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let a = val(chunk[0])?;
        let b = if chunk.len() > 1 && chunk[1] != b'=' {
            val(chunk[1])?
        } else {
            0
        };
        out.push((a << 2) | (b >> 4));
        if chunk.len() > 2 && chunk[2] != b'=' {
            let c = val(chunk[2])?;
            out.push((b << 4) | (c >> 2));
            if chunk.len() > 3 && chunk[3] != b'=' {
                let d = val(chunk[3])?;
                out.push((c << 6) | d);
            }
        }
    }
    Ok(out)
}

/// Exact check for Tauri's internal bundled asset origin (`tauri://localhost`
/// or `http(s)://tauri.localhost`). Uses full host equality so CVE-2026-42184
/// first-label tricks (`http://tauri.evil.com`) never match.
pub fn is_local_app_url(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "http" | "https" => url.host_str() == Some("tauri.localhost"),
        _ => false,
    }
}

fn check_origin(webview: &tauri::Webview, state: &AppState) -> Result<(), String> {
    let url = webview.url().map_err(|e| e.to_string())?;
    if is_local_app_url(&url) {
        return Ok(());
    }
    let origin = url.origin().ascii_serialization();
    let guard = state
        .configured_origin
        .read()
        .map_err(|_| "config_lock_poisoned".to_string())?;
    if let Some(configured) = guard.as_deref() {
        if origin_is_allowed(configured, &origin) {
            return Ok(());
        }
    }
    Err("forbidden_origin".to_string())
}

fn check_local_only(webview: &tauri::Webview) -> Result<(), String> {
    let url = webview.url().map_err(|e| e.to_string())?;
    if is_local_app_url(&url) {
        Ok(())
    } else {
        Err("local_control_center_only".to_string())
    }
}

/// The payload the Odoo addon's handler sends (API contract v1).
/// Field names are camelCase on the wire, matching the addon.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintPdfRequest {
    pub idempotency_key: String,
    pub report_name: String,
    pub printer_pattern: String,
    pub mode: String,
    pub paper: Option<String>,
    pub copies: u16,
    pub scale_policy: Option<String>,
    #[serde(default)]
    pub duplex: bool,
    #[serde(default = "default_true")]
    pub color: bool,
    #[serde(default)]
    pub tray: Option<String>,
    #[serde(default)]
    pub media_roll_width_mm: Option<f64>,
    #[allow(dead_code)]
    pub url: String,
    #[serde(deserialize_with = "deserialize_bytes")]
    pub bytes: Vec<u8>,
}

fn default_true() -> bool {
    true
}

#[tauri::command]
pub async fn print_pdf(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    req: PrintPdfRequest,
) -> Result<String, String> {
    check_origin(&webview, &state)?;
    // Validate everything that does not need the document BEFORE any
    // parser touches untrusted bytes (size, copies, key shape).
    if !print_core::security::valid_idempotency_key(&req.idempotency_key) {
        return Err("bad_idempotency_key".to_string());
    }
    print_core::security::validate_payload_limits(req.bytes.len(), req.copies)
        .map_err(|e| format!("{e:?}"))?;
    let info = print_render::parse_pdf_info(&req.bytes).map_err(|e| e.to_string())?;
    let pages = info.pages.len() as u32;
    if pages == 0 {
        return Err("empty_document".to_string());
    }
    for p in &info.pages {
        validate_job_input(req.bytes.len(), pages, req.copies, p.size)
            .map_err(|e| format!("{e:?}"))?;
    }
    let job = Job {
        id: req.idempotency_key.clone(),
        idempotency_key: req.idempotency_key.clone(),
        printer: req.printer_pattern.clone(),
        report: Some(req.report_name.clone()),
        pages,
        bytes: req.bytes.len() as u64,
        priority: if req.mode == "escpos" { 10 } else { 0 },
        state: JobState::Queued,
        attempts: 0,
        error_code: None,
    };
    let opts = JobOptions {
        mode: req.mode.clone(),
        paper: req.paper.clone(),
        copies: req.copies.max(1),
        scale_policy: req.scale_policy.clone(),
        duplex: req.duplex,
        color: req.color,
        cut: true,
        tray: req.tray.clone(),
        media_roll_width_mm: req.media_roll_width_mm,
    };
    let id = {
        let mut queue = state
            .queue
            .lock()
            .map_err(|_| "queue_poisoned".to_string())?;
        // Duplicate submissions (same idempotency key) return the
        // original job untouched: never overwrite its payload/options.
        // Job + payload + options land in ONE SQLite transaction:
        // a crash mid-submission leaves nothing behind, and a
        // duplicate returns the original job untouched.
        match queue
            .enqueue_with_payload(&job, "pdf", &req.bytes, &opts)
            .map_err(|e| e.to_string())?
        {
            Some(new_id) => new_id,
            None => queue
                .job_id_for_key(&job.idempotency_key)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "queue_inconsistent".to_string())?,
        }
    };
    state.notify_worker();
    Ok(id)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintTextRequest {
    pub idempotency_key: String,
    pub report_name: String,
    pub printer_pattern: String,
    pub text: String,
    pub cut: bool,
    #[serde(default)]
    pub copies: Option<u16>,
}

#[tauri::command]
pub fn print_text(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    req: PrintTextRequest,
) -> Result<String, String> {
    check_origin(&webview, &state)?;
    if req.text.is_empty() || req.text.len() > 1_048_576 {
        return Err("bad_text_size".to_string());
    }
    let copies = req.copies.unwrap_or(1);
    validate_job_input(
        req.text.len(),
        1,
        copies,
        print_core::PageSize::roll_80(200.0),
    )
    .map_err(|e| format!("{e:?}"))?;
    let mut stream = print_raw::EscPos::new();
    for line in req.text.lines() {
        stream = stream.text_line(line);
    }
    stream = stream.feed_lines(3);
    if req.cut {
        stream = stream.cut_partial();
    }
    let bytes = stream.bytes().to_vec();
    let job = Job {
        id: req.idempotency_key.clone(),
        idempotency_key: req.idempotency_key.clone(),
        printer: req.printer_pattern.clone(),
        report: Some(req.report_name.clone()),
        pages: 1,
        bytes: bytes.len() as u64,
        priority: 10,
        state: JobState::Queued,
        attempts: 0,
        error_code: None,
    };
    let opts = JobOptions {
        mode: "escpos".to_string(),
        paper: None,
        copies: copies.max(1),
        scale_policy: None,
        duplex: false,
        color: false,
        cut: req.cut,
        tray: None,
        media_roll_width_mm: None,
    };
    let id = {
        let mut queue = state
            .queue
            .lock()
            .map_err(|_| "queue_poisoned".to_string())?;
        // Duplicate submissions (same idempotency key) return the
        // original job untouched: never overwrite its payload/options.
        // Job + payload + options land in ONE SQLite transaction:
        // a crash mid-submission leaves nothing behind, and a
        // duplicate returns the original job untouched.
        match queue
            .enqueue_with_payload(&job, "escpos", &bytes, &opts)
            .map_err(|e| e.to_string())?
        {
            Some(new_id) => new_id,
            None => queue
                .job_id_for_key(&job.idempotency_key)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "queue_inconsistent".to_string())?,
        }
    };
    state.notify_worker();
    Ok(id)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintImageRequest {
    pub idempotency_key: String,
    pub report_name: String,
    pub printer_pattern: String,
    pub paper: Option<String>,
    #[serde(default)]
    pub media_roll_width_mm: Option<f64>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub copies: Option<u16>,
    #[serde(deserialize_with = "deserialize_bytes")]
    pub bytes: Vec<u8>,
}

#[tauri::command]
pub async fn print_image(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    req: PrintImageRequest,
) -> Result<String, String> {
    check_origin(&webview, &state)?;
    let copies = req.copies.unwrap_or(1);
    // Validate BEFORE decoding untrusted image bytes (see print_pdf).
    if !print_core::security::valid_idempotency_key(&req.idempotency_key) {
        return Err("bad_idempotency_key".to_string());
    }
    print_core::security::validate_payload_limits(req.bytes.len(), copies)
        .map_err(|e| format!("{e:?}"))?;
    let _img = print_render::decode_png(&req.bytes).map_err(|e| e.to_string())?;
    validate_job_input(
        req.bytes.len(),
        1,
        copies,
        print_core::PageSize::roll_80(200.0),
    )
    .map_err(|e| format!("{e:?}"))?;
    let job = Job {
        id: req.idempotency_key.clone(),
        idempotency_key: req.idempotency_key.clone(),
        printer: req.printer_pattern.clone(),
        report: Some(req.report_name.clone()),
        pages: 1,
        bytes: req.bytes.len() as u64,
        priority: 10,
        state: JobState::Queued,
        attempts: 0,
        error_code: None,
    };
    let opts = JobOptions {
        mode: req.mode.unwrap_or_else(|| "escpos".to_string()),
        paper: req.paper,
        copies: copies.max(1),
        scale_policy: Some("custom_allowed".to_string()),
        duplex: false,
        color: false,
        cut: true,
        tray: None,
        media_roll_width_mm: req.media_roll_width_mm,
    };
    let id = {
        let mut queue = state
            .queue
            .lock()
            .map_err(|_| "queue_poisoned".to_string())?;
        // Duplicate submissions (same idempotency key) return the
        // original job untouched: never overwrite its payload/options.
        // Job + payload + options land in ONE SQLite transaction:
        // a crash mid-submission leaves nothing behind, and a
        // duplicate returns the original job untouched.
        match queue
            .enqueue_with_payload(&job, "image-png", &req.bytes, &opts)
            .map_err(|e| e.to_string())?
        {
            Some(new_id) => new_id,
            None => queue
                .job_id_for_key(&job.idempotency_key)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "queue_inconsistent".to_string())?,
        }
    };
    state.notify_worker();
    Ok(id)
}

#[tauri::command]
pub fn print_status(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<String, String> {
    check_origin(&webview, &state)?;
    let queue = state
        .queue
        .lock()
        .map_err(|_| "queue_poisoned".to_string())?;
    let job = queue.get(&job_id).map_err(|e| e.to_string())?;
    Ok(job.state.as_str().to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDetailResponse {
    pub id: String,
    pub state: String,
    pub error_code: Option<String>,
    pub printer: String,
    pub report: Option<String>,
    pub pages: u32,
    pub bytes: u64,
    pub attempts: u32,
}

#[tauri::command]
pub fn job_detail(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<JobDetailResponse, String> {
    check_origin(&webview, &state)?;
    let queue = state
        .queue
        .lock()
        .map_err(|_| "queue_poisoned".to_string())?;
    let job = queue.get(&job_id).map_err(|e| e.to_string())?;
    Ok(JobDetailResponse {
        id: job.id,
        state: job.state.as_str().to_string(),
        error_code: job.error_code,
        printer: job.printer,
        report: job.report,
        pages: job.pages,
        bytes: job.bytes,
        attempts: job.attempts,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppOverviewResponse {
    pub app_version: String,
    pub os: String,
    pub odoo_origin: Option<String>,
    pub locked_by_policy: bool,
    pub auto_reprint: bool,
    pub pdfium_ready: bool,
    pub webview_version: String,
    pub environment: Option<serde_json::Value>,
    pub printers: Vec<PrinterInfo>,
    pub default_printer: Option<String>,
    pub state_counts: Vec<(String, u64)>,
    pub recent_jobs: Vec<JobSummary>,
}

#[tauri::command]
pub async fn app_overview(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AppOverviewResponse, String> {
    check_origin(&webview, &state)?;
    let pdfium_ready = pdfium_ready();
    let (printers, default_printer) = cached_printers(&state)?;
    let origin = state
        .configured_origin
        .read()
        .map_err(|_| "config_lock_poisoned".to_string())?
        .clone();
    let auto_reprint = *state
        .auto_reprint
        .read()
        .map_err(|_| "config_lock_poisoned".to_string())?;
    let bundle = {
        let queue = state
            .queue
            .lock()
            .map_err(|_| "queue_poisoned".to_string())?;
        build_bundle_from_queue(&queue).map_err(|e| e.to_string())?
    };
    Ok(AppOverviewResponse {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        odoo_origin: origin,
        locked_by_policy: state.locked_by_policy,
        auto_reprint,
        pdfium_ready,
        webview_version: tauri::webview_version().unwrap_or_else(|_| "unknown".to_string()),
        environment: state
            .environment
            .read()
            .ok()
            .and_then(|g| g.clone())
            .and_then(|raw| serde_json::from_str(&raw).ok()),
        printers,
        default_printer,
        state_counts: bundle.state_counts,
        recent_jobs: bundle.recent_jobs,
    })
}

#[tauri::command]
pub async fn list_printers(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<Vec<PrinterInfo>, String> {
    check_origin(&webview, &state)?;
    let (printers, _default) = cached_printers(&state)?;
    Ok(printers)
}

#[tauri::command]
pub async fn printer_forms(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    printer: String,
) -> Result<Vec<Form>, String> {
    check_origin(&webview, &state)?;
    print_win::paper_forms(&printer).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn printer_capabilities(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    printer: String,
) -> Result<print_core::PrinterCapabilities, String> {
    check_origin(&webview, &state)?;
    if printer.trim().is_empty() {
        return Err("printer_required".to_string());
    }
    print_win::printer_capabilities(printer.trim()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn run_test_print(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    printer: String,
    kind: String,
    paper: Option<String>,
) -> Result<String, String> {
    check_origin(&webview, &state)?;
    if printer.trim().is_empty() {
        return Err("printer_required".to_string());
    }
    // Raw calibration pages (ESC/POS / ZPL / TSPL) sent to a virtual
    // PDF/XPS writer produce a corrupt file instead of a page; refuse
    // up front with the reason instead of enqueueing a doomed job.
    let raw_kind = matches!(
        kind.as_str(),
        "thermal-80mm" | "thermal-58mm" | "escpos" | "zpl-label" | "zpl" | "tspl-label" | "tspl"
    );
    if raw_kind && !printer.contains("://") {
        if let Ok(printers) = print_win::discover_printers() {
            if let Some(found) = printers.iter().find(|p| p.name == printer.trim()) {
                if found.kind == PrinterKind::Virtual {
                    return Err(
                        "This calibration sends raw printer language (ESC/POS / ZPL / TSPL) and cannot go to a virtual PDF/XPS printer - it would create a corrupt file. Choose a physical printer."
                            .to_string(),
                    );
                }
            }
        }
    }
    let key = format!(
        "cal-{}-{}",
        kind.replace(|c: char| !c.is_ascii_alphanumeric(), "-"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(1)
    );
    let (payload_kind, payload_bytes, opts, prio) = match kind.as_str() {
        "thermal-80mm" | "thermal-58mm" | "escpos" => {
            let is_58 = kind == "thermal-58mm"
                || paper
                    .as_deref()
                    .map(|p| print_render::receipt_dots_for_paper(p) == 384)
                    .unwrap_or(false);
            let bm = if is_58 {
                print_render::calibration::calibration_thermal_58mm()
            } else {
                print_render::calibration::calibration_thermal_80mm()
            };
            let stream = print_raw::EscPos::new()
                .raster(&bm)
                .feed_lines(4)
                .cut_partial();
            let default_paper = if is_58 { "58mm" } else { "80mm" };
            (
                "escpos",
                stream.bytes().to_vec(),
                JobOptions {
                    mode: "escpos".to_string(),
                    paper: Some(paper.unwrap_or_else(|| default_paper.to_string())),
                    copies: 1,
                    scale_policy: Some("custom_allowed".to_string()),
                    duplex: false,
                    color: false,
                    cut: true,
                    tray: None,
                    media_roll_width_mm: None,
                },
                10,
            )
        }
        "zpl-label" | "zpl" => {
            let bm = print_render::calibration::calibration_thermal_80mm();
            let zpl = print_raw::zpl_label_with_options(&bm, 0, 0, 1);
            (
                "zpl",
                zpl.into_bytes(),
                JobOptions {
                    mode: "zpl".to_string(),
                    paper: Some(paper.unwrap_or_else(|| "100x150mm".to_string())),
                    copies: 1,
                    scale_policy: Some("custom_allowed".to_string()),
                    duplex: false,
                    color: false,
                    cut: false,
                    tray: None,
                    media_roll_width_mm: None,
                },
                5,
            )
        }
        "tspl-label" | "tspl" => {
            let bm = print_render::calibration::calibration_thermal_80mm();
            let tspl = print_raw::tspl::TsplLabel::new(100, 150, 3)
                .bitmap(0, 0, &bm)
                .text(40, 20, "Yaseir Print TSPL Calibration")
                .print(1, 1);
            (
                "tspl",
                tspl,
                JobOptions {
                    mode: "tspl".to_string(),
                    paper: Some(paper.unwrap_or_else(|| "100x150mm".to_string())),
                    copies: 1,
                    scale_policy: Some("custom_allowed".to_string()),
                    duplex: false,
                    color: false,
                    cut: false,
                    tray: None,
                    media_roll_width_mm: None,
                },
                5,
            )
        }
        "pdf-a4" | "pdf" | "gdi" => {
            let pdf = print_render::calibration::calibration_pdf();
            (
                "pdf",
                pdf,
                JobOptions {
                    mode: "gdi".to_string(),
                    paper: Some(paper.unwrap_or_else(|| "A4".to_string())),
                    copies: 1,
                    scale_policy: Some("custom_allowed".to_string()),
                    duplex: false,
                    color: false,
                    cut: false,
                    tray: None,
                    media_roll_width_mm: None,
                },
                0,
            )
        }
        _ => return Err("unsupported_test_kind".to_string()),
    };
    let job = Job {
        id: key.clone(),
        idempotency_key: key,
        printer: printer.trim().to_string(),
        report: Some(format!("calibration.{kind}")),
        pages: 1,
        bytes: payload_bytes.len() as u64,
        priority: prio,
        state: JobState::Queued,
        attempts: 0,
        error_code: None,
    };
    let id = {
        let mut queue = state
            .queue
            .lock()
            .map_err(|_| "queue_poisoned".to_string())?;
        match queue
            .enqueue_with_payload(&job, payload_kind, &payload_bytes, &opts)
            .map_err(|e| e.to_string())?
        {
            Some(id) => id,
            None => queue
                .job_id_for_key(&job.idempotency_key)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "queue_inconsistent".to_string())?,
        }
    };
    state.notify_worker();
    Ok(id)
}

#[tauri::command]
pub fn retry_job_cmd(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), String> {
    check_origin(&webview, &state)?;
    check_local_only(&webview)?;
    {
        let mut queue = state
            .queue
            .lock()
            .map_err(|_| "queue_poisoned".to_string())?;
        queue.retry_job(&job_id).map_err(|e| e.to_string())?;
    }
    state.notify_worker();
    Ok(())
}

#[tauri::command]
pub fn cancel_job_cmd(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), String> {
    check_origin(&webview, &state)?;
    check_local_only(&webview)?;
    let mut queue = state
        .queue
        .lock()
        .map_err(|_| "queue_poisoned".to_string())?;
    queue.cancel_job(&job_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_finished_cmd(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    check_origin(&webview, &state)?;
    check_local_only(&webview)?;
    let mut queue = state
        .queue
        .lock()
        .map_err(|_| "queue_poisoned".to_string())?;
    queue.clear_finished().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_diagnostics(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<String, String> {
    check_origin(&webview, &state)?;
    check_local_only(&webview)?;
    let queue = state
        .queue
        .lock()
        .map_err(|_| "queue_poisoned".to_string())?;
    let bundle = build_bundle_from_queue(&queue).map_err(|e| e.to_string())?;
    let mut value: serde_json::Value =
        serde_json::from_str(&bundle_to_json(&bundle)).unwrap_or(serde_json::json!({}));
    // The bundle is built by print-core and stamps its own crate
    // version; report the desktop app's version instead.
    value["app_version"] = serde_json::json!(env!("CARGO_PKG_VERSION"));
    value["webview_version"] =
        serde_json::json!(tauri::webview_version().unwrap_or_else(|_| "unknown".to_string()));
    if let Some(raw) = state.environment.read().ok().and_then(|g| g.clone()) {
        if let Ok(env) = serde_json::from_str::<serde_json::Value>(&raw) {
            value["environment"] = env;
        }
    }
    Ok(value.to_string())
}

/// Keep the signed-in Odoo session across app restarts. Odoo's
/// `session_id` cookie is a session cookie, which WebView2 discards
/// when the app closes, so the user had to sign in again on every
/// launch. After a page on the configured origin reports in (which
/// happens right after the post-login redirect), rewrite the cookie
/// in the webview's own cookie store with a long expiry. The value
/// never leaves the webview store; nothing is persisted by Rust.
/// Best-effort by design: any failure just keeps the old behaviour.
fn persist_session_cookie(webview: &tauri::Webview, state: &AppState) {
    let origin = match state.configured_origin.read() {
        Ok(guard) => guard.clone(),
        Err(_) => None,
    };
    let Some(origin) = origin else { return };
    let Ok(url) = origin.parse::<tauri::Url>() else {
        return;
    };
    let Ok(cookies) = webview.cookies_for_url(url) else {
        return;
    };
    for cookie in cookies {
        if cookie.name() != "session_id" {
            continue;
        }
        // A cookie with neither an expiry nor a max-age is a session
        // cookie: make it permanent in place and write it back.
        if cookie.expires().is_none() && cookie.max_age().is_none() {
            let mut persistent = cookie;
            persistent.make_permanent();
            let _ = webview.set_cookie(persistent);
        }
    }
}

/// Identity for lightweight chrome (the toolbar webview): app version
/// and the configured origin, without the heavyweight overview data
/// (no printer enumeration, no queue bundle).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIdentity {
    pub app_version: String,
    pub odoo_origin: Option<String>,
}

#[tauri::command]
pub fn app_identity(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AppIdentity, String> {
    check_origin(&webview, &state)?;
    let origin = state
        .configured_origin
        .read()
        .map_err(|_| "config_lock_poisoned".to_string())?
        .clone();
    Ok(AppIdentity {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        odoo_origin: origin,
    })
}

/// Browser-style navigation for the toolbar webview: history back on
/// the main webview. Odoo is an SPA whose screens stack routes with no
/// visible way back; these give the user the browser controls back.
#[tauri::command]
pub fn nav_back(app: tauri::AppHandle, webview: tauri::Webview) -> Result<(), String> {
    check_local_only(&webview)?;
    let main = app
        .get_webview("main")
        .ok_or_else(|| "main_webview_missing".to_string())?;
    main.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
pub fn nav_forward(app: tauri::AppHandle, webview: tauri::Webview) -> Result<(), String> {
    check_local_only(&webview)?;
    let main = app
        .get_webview("main")
        .ok_or_else(|| "main_webview_missing".to_string())?;
    main.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
pub fn nav_reload(app: tauri::AppHandle, webview: tauri::Webview) -> Result<(), String> {
    check_local_only(&webview)?;
    let main = app
        .get_webview("main")
        .ok_or_else(|| "main_webview_missing".to_string())?;
    main.reload().map_err(|e| e.to_string())
}

/// Store the page's environment probe report (JS features, storage,
/// secure-context facts). Reported by the injected bridge on Odoo
/// pages and by the Control Center; shown in the overview and folded
/// into the diagnostics bundle.
#[tauri::command]
pub async fn report_environment(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    report: String,
) -> Result<(), String> {
    check_origin(&webview, &state)?;
    if report.len() > 8192 {
        return Err("environment_report_too_large".to_string());
    }
    {
        let mut guard = state
            .environment
            .write()
            .map_err(|_| "environment_lock_poisoned".to_string())?;
        *guard = Some(report);
    }
    // A page on the configured origin just loaded: if the user is
    // signed in, keep that session alive across restarts.
    if !is_local_app_url(&webview.url().map_err(|e| e.to_string())?) {
        persist_session_cookie(&webview, &state);
    }
    Ok(())
}

/// Self-heal for a poisoned webview profile: wipe ALL browsing data
/// (cache, cookies, storage, service workers) for this app, then
/// reload the configured Odoo origin with a clean slate. The user
/// signs in again afterwards. This is the one-click cure for the
/// "white page after login" class of failures.
#[tauri::command]
pub fn clear_web_data_cmd(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_origin(&webview, &state)?;
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main_window_missing".to_string())?;
    window
        .clear_all_browsing_data()
        .map_err(|e| e.to_string())?;
    let origin = state
        .configured_origin
        .read()
        .map_err(|_| "config_lock_poisoned".to_string())?
        .clone();
    if let Some(origin) = origin {
        let url = origin.parse::<tauri::Url>().map_err(|e| e.to_string())?;
        window.navigate(url).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn save_origin_cmd(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    origin: String,
    auto_reprint: Option<bool>,
) -> Result<String, String> {
    check_local_only(&webview)?;
    if state.locked_by_policy {
        return Err("locked_by_policy".to_string());
    }
    let current_auto = *state
        .auto_reprint
        .read()
        .map_err(|_| "config_lock_poisoned".to_string())?;
    let next_auto = auto_reprint.unwrap_or(current_auto);
    let validated = config::save_origin(&state.config_dir, &origin, next_auto)?;
    {
        let mut guard = state
            .configured_origin
            .write()
            .map_err(|_| "config_lock_poisoned".to_string())?;
        *guard = Some(validated.clone());
    }
    {
        let mut auto_guard = state
            .auto_reprint
            .write()
            .map_err(|_| "config_lock_poisoned".to_string())?;
        *auto_guard = next_auto;
    }
    Ok(validated)
}

#[tauri::command]
pub fn open_control_center(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_origin(&webview, &state)?;
    let local_url: tauri::Url = if cfg!(windows) {
        "http://tauri.localhost/index.html"
            .parse()
            .map_err(|_| "bad_local_url".to_string())?
    } else {
        "tauri://localhost/index.html"
            .parse()
            .map_err(|_| "bad_local_url".to_string())?
    };
    webview.navigate(local_url).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_odoo_origin(webview: tauri::Webview, state: State<'_, AppState>) -> Result<(), String> {
    check_origin(&webview, &state)?;
    let origin = state
        .configured_origin
        .read()
        .map_err(|_| "config_lock_poisoned".to_string())?
        .clone()
        .ok_or_else(|| "no_origin_configured".to_string())?;
    let url: tauri::Url = origin.parse().map_err(|_| "bad_origin_url".to_string())?;
    webview.navigate(url).map_err(|e| e.to_string())
}

// --- Background Print Worker & Hardware Dispatch ----------------------

struct NativePrintBackend {
    db_path: String,
}

impl NativePrintBackend {
    fn status_code(s: PrinterStatus) -> &'static str {
        match s {
            PrinterStatus::Offline => "PRINTER_OFFLINE",
            PrinterStatus::PaperOut => "PAPER_OUT",
            PrinterStatus::PaperJam => "PAPER_JAM",
            PrinterStatus::DoorOpen => "DOOR_OPEN",
            PrinterStatus::NoToner => "NO_TONER",
            PrinterStatus::Paused => "PRINTER_PAUSED",
            PrinterStatus::Error => "PRINTER_ERROR",
            _ => "PRINTER_BUSY",
        }
    }

    fn receipt_width_dots(
        printer: &str,
        paper_override: Option<&str>,
    ) -> Result<u32, BackendError> {
        if let Some(paper) = paper_override {
            let dots = print_render::receipt_dots_for_paper(paper);
            // An explicit paper choice is verified against the driver's
            // reported roll forms when they are available: rendering an
            // 80mm receipt for a 58mm roll (or the reverse) clips or
            // scales the customer's receipt.
            if let Ok(caps) = print_win::printer_capabilities(printer) {
                let rolls: Vec<f64> = caps
                    .forms
                    .iter()
                    .filter(|f| f.roll)
                    .map(|f| f.size.width_mm)
                    .collect();
                if !rolls.is_empty() {
                    let nominal_mm = dots as f64 * 25.4 / 203.0 + 8.0;
                    if !rolls.iter().any(|w| (w - nominal_mm).abs() <= 6.0) {
                        return Err(BackendError::fatal(
                            "PAPER_MISMATCH",
                            &format!(
                                "requested paper {paper:?} does not match the roll forms reported by the driver: {rolls:?} mm"
                            ),
                        ));
                    }
                }
            }
            return Ok(dots);
        }
        let capabilities = print_win::printer_capabilities(printer).map_err(|error| {
            BackendError::fatal(
                "PRINTER_CAPABILITIES_UNAVAILABLE",
                &format!("The Windows driver did not expose printer capabilities: {error}"),
            )
        })?;
        let roll = capabilities.forms.iter().find(|form| form.roll).ok_or_else(|| {
            BackendError::fatal(
                "ROLL_MEDIA_NOT_REPORTED",
                "The printer driver did not report a continuous/receipt paper form. Install or update the vendor driver; Native Print will not assume 58mm or 80mm.",
            )
        })?;
        print_render::receipt_dots_for_width_mm(roll.size.width_mm).ok_or_else(|| {
            BackendError::fatal(
                "ROLL_WIDTH_UNSUPPORTED",
                &format!(
                    "The detected roll width ({:.1} mm) has no verified ESC/POS print-head profile.",
                    roll.size.width_mm
                ),
            )
        })
    }

    fn parse_scale_policy(s: Option<&str>) -> ScalePolicy {
        match s.unwrap_or("custom_allowed") {
            "strict" => ScalePolicy::Strict,
            "nearest_larger_centered" => ScalePolicy::NearestLargerCentered,
            "nearest_larger_fit" => ScalePolicy::NearestLargerFit,
            _ => ScalePolicy::CustomAllowed,
        }
    }

    fn parse_host_port(
        spec: &str,
        default_port: u16,
    ) -> Result<(String, u16, String), BackendError> {
        let rest = spec
            .strip_prefix("tcp://")
            .or_else(|| spec.strip_prefix("raw://"))
            .or_else(|| spec.strip_prefix("ipp://"))
            .or_else(|| spec.strip_prefix("http://"))
            .unwrap_or(spec);
        let (host_port, path) = match rest.split_once('/') {
            Some((hp, p)) => (hp, format!("/{p}")),
            None => (rest, "/ipp/print".to_string()),
        };
        let (host, port) = if let Some(after_open) = host_port.strip_prefix('[') {
            let (ipv6, after_close) = after_open.split_once(']').ok_or_else(|| {
                BackendError::fatal("BAD_PRINTER_ADDRESS", "unclosed IPv6 bracket")
            })?;
            let port = if after_close.is_empty() {
                default_port
            } else if let Some(p) = after_close.strip_prefix(':') {
                p.parse::<u16>().map_err(|_| {
                    BackendError::fatal("BAD_PRINTER_ADDRESS", "invalid port in printer address")
                })?
            } else {
                return Err(BackendError::fatal(
                    "BAD_PRINTER_ADDRESS",
                    "invalid IPv6 host:port",
                ));
            };
            (ipv6.to_string(), port)
        } else {
            match host_port.rsplit_once(':') {
                Some((h, p)) => {
                    let parsed = p.parse::<u16>().map_err(|_| {
                        BackendError::fatal(
                            "BAD_PRINTER_ADDRESS",
                            "invalid port in printer address",
                        )
                    })?;
                    (h.to_string(), parsed)
                }
                None => (host_port.to_string(), default_port),
            }
        };
        if host.is_empty() {
            return Err(BackendError::fatal(
                "BAD_PRINTER_ADDRESS",
                "empty printer host",
            ));
        }
        Ok((host, port, path))
    }

    fn pdf_to_raw_stream(
        pdf_bytes: &[u8],
        mode: &str,
        paper: Option<&str>,
        target_width_dots: Option<u32>,
        copies: u16,
        cut: bool,
        label_box: Option<(u32, u32)>,
    ) -> Result<Vec<u8>, BackendError> {
        let pdfium = print_render::Pdfium::load()
            .map_err(|e| BackendError::fatal("PDFIUM_LOAD_ERROR", &e.to_string()))?;
        let doc = pdfium
            .load_document(pdf_bytes)
            .map_err(|e| BackendError::fatal("PDF_INVALID", &e.to_string()))?;
        let count = doc.page_count();
        if count == 0 {
            return Err(BackendError::fatal("EMPTY_DOCUMENT", "PDF has 0 pages"));
        }
        match mode {
            "escpos" => {
                let target_w = target_width_dots
                    .or_else(|| paper.map(print_render::receipt_dots_for_paper))
                    .ok_or_else(|| {
                        BackendError::fatal(
                            "AUTO_MEDIA_UNAVAILABLE",
                            "No driver-reported roll width is available for ESC/POS rendering.",
                        )
                    })?;
                let mut stream = print_raw::EscPos::new();
                for _ in 0..copies.max(1) {
                    for i in 0..count {
                        let page = doc
                            .render_page(i, RenderMode::Grayscale, 203)
                            .map_err(|e| BackendError::fatal("RENDER_FAILED", &e.to_string()))?;
                        let canvas = print_render::bitmap::Canvas {
                            width: page.width_px,
                            height: page.height_px,
                            px: page.pixels,
                        };
                        let scaled = print_render::scale_to_width(&canvas, target_w);
                        let mono = scaled.to_mono(print_render::image::RECEIPT_THRESHOLD);
                        stream = stream.raster(&mono).feed_lines(4);
                        if cut {
                            stream = stream.cut_partial();
                        }
                    }
                }
                Ok(stream.bytes().to_vec())
            }
            "zpl" => {
                let mut out = String::new();
                for i in 0..count {
                    let page = doc
                        .render_page(i, RenderMode::Threshold, 203)
                        .map_err(|e| BackendError::fatal("RENDER_FAILED", &e.to_string()))?;
                    let w_aligned = page.width_px.div_ceil(8) * 8;
                    let bm = MonoBitmap::from_packed(w_aligned, page.height_px, page.pixels)
                        .map_err(|e| BackendError::fatal("BITMAP_PACK_ERROR", &e.to_string()))?;
                    let bm = match label_box {
                        Some((w, h)) => print_render::bitmap::fit_mono_to_box(&bm, w, h),
                        None => bm,
                    };
                    out.push_str(&print_raw::zpl_label_with_options(&bm, 0, 0, copies));
                }
                Ok(out.into_bytes())
            }
            "tspl" => {
                let mut out = Vec::new();
                for i in 0..count {
                    let page = doc
                        .render_page(i, RenderMode::Threshold, 203)
                        .map_err(|e| BackendError::fatal("RENDER_FAILED", &e.to_string()))?;
                    let w_aligned = page.width_px.div_ceil(8) * 8;
                    let bm = MonoBitmap::from_packed(w_aligned, page.height_px, page.pixels)
                        .map_err(|e| BackendError::fatal("BITMAP_PACK_ERROR", &e.to_string()))?;
                    let bm = match label_box {
                        Some((w, h)) => print_render::bitmap::fit_mono_to_box(&bm, w, h),
                        None => bm,
                    };
                    let w_mm = ((bm.width_dots as f64) * 25.4 / (page.dpi.max(1) as f64))
                        .round()
                        .max(20.0) as u32;
                    let h_mm = ((bm.height_dots as f64) * 25.4 / (page.dpi.max(1) as f64))
                        .round()
                        .max(20.0) as u32;
                    let tspl = print_raw::tspl::TsplLabel::new(w_mm, h_mm, 3)
                        .bitmap(0, 0, &bm)
                        .print(1, copies.max(1) as u32);
                    out.extend_from_slice(&tspl);
                }
                Ok(out)
            }
            "epl" => {
                let mut out = Vec::new();
                for i in 0..count {
                    let page = doc
                        .render_page(i, RenderMode::Threshold, 203)
                        .map_err(|e| BackendError::fatal("RENDER_FAILED", &e.to_string()))?;
                    let w_aligned = page.width_px.div_ceil(8) * 8;
                    let bm = MonoBitmap::from_packed(w_aligned, page.height_px, page.pixels)
                        .map_err(|e| BackendError::fatal("BITMAP_PACK_ERROR", &e.to_string()))?;
                    let bm = match label_box {
                        Some((w, h)) => print_render::bitmap::fit_mono_to_box(&bm, w, h),
                        None => bm,
                    };
                    out.extend_from_slice(&print_raw::epl::epl_label_copies(&bm, copies));
                }
                Ok(out)
            }
            _ => Err(BackendError::fatal(
                "UNKNOWN_MODE",
                &format!("unsupported print mode {mode:?} for a PDF document"),
            )),
        }
    }

    fn png_to_raw_stream(
        png_bytes: &[u8],
        mode: &str,
        paper: Option<&str>,
        target_width_dots: Option<u32>,
        copies: u16,
        label_box: Option<(u32, u32)>,
    ) -> Result<Vec<u8>, BackendError> {
        match mode {
            "zpl" => {
                let canvas = print_render::decode_png(png_bytes)
                    .map_err(|e| BackendError::fatal("IMAGE_DECODE_ERROR", &e.to_string()))?;
                let mono = canvas.to_mono(print_render::image::RECEIPT_THRESHOLD);
                let mono = match label_box {
                    Some((w, h)) => print_render::bitmap::fit_mono_to_box(&mono, w, h),
                    None => mono,
                };
                Ok(print_raw::zpl_label_with_options(&mono, 0, 0, copies).into_bytes())
            }
            "tspl" => {
                let canvas = print_render::decode_png(png_bytes)
                    .map_err(|e| BackendError::fatal("IMAGE_DECODE_ERROR", &e.to_string()))?;
                let mono = canvas.to_mono(print_render::image::RECEIPT_THRESHOLD);
                let mono = match label_box {
                    Some((w, h)) => print_render::bitmap::fit_mono_to_box(&mono, w, h),
                    None => mono,
                };
                let w_mm = ((mono.width_dots as f64) * 25.4 / 203.0).round().max(20.0) as u32;
                let h_mm = ((mono.height_dots as f64) * 25.4 / 203.0).round().max(20.0) as u32;
                Ok(print_raw::tspl::TsplLabel::new(w_mm, h_mm, 3)
                    .bitmap(0, 0, &mono)
                    .print(1, copies.max(1) as u32))
            }
            "epl" => {
                let canvas = print_render::decode_png(png_bytes)
                    .map_err(|e| BackendError::fatal("IMAGE_DECODE_ERROR", &e.to_string()))?;
                let mono = canvas.to_mono(print_render::image::RECEIPT_THRESHOLD);
                let mono = match label_box {
                    Some((w, h)) => print_render::bitmap::fit_mono_to_box(&mono, w, h),
                    None => mono,
                };
                Ok(print_raw::epl::epl_label_copies(&mono, copies))
            }
            "escpos" => {
                let target_width_dots = target_width_dots
                    .or_else(|| paper.map(print_render::receipt_dots_for_paper))
                    .ok_or_else(|| {
                        BackendError::fatal(
                            "AUTO_MEDIA_UNAVAILABLE",
                            "No driver-reported roll width is available for ESC/POS rendering.",
                        )
                    })?;
                let single =
                    print_render::receipt_png_to_escpos_width(png_bytes, target_width_dots)
                        .map_err(|e| BackendError::fatal("IMAGE_DECODE_ERROR", &e.to_string()))?;
                let mut out = Vec::with_capacity(single.len() * copies.max(1) as usize);
                for _ in 0..copies.max(1) {
                    out.extend_from_slice(&single);
                }
                Ok(out)
            }
            _ => Err(BackendError::fatal(
                "UNKNOWN_MODE",
                &format!("unsupported print mode {mode:?} for an image"),
            )),
        }
    }
}

impl PrintBackend for NativePrintBackend {
    fn print(&mut self, job: &Job) -> Result<(), BackendError> {
        let q = Queue::open(&self.db_path)
            .map_err(|e| BackendError::retryable("QUEUE_DB_ERROR", &e.to_string()))?;
        let (kind, data) = q
            .payload(&job.id)
            .map_err(|e| BackendError::retryable("QUEUE_DB_ERROR", &e.to_string()))?
            .ok_or_else(|| BackendError::fatal("MISSING_PAYLOAD", "job payload not found"))?;
        let opts = q.options(&job.id).unwrap_or_default();
        let paper = opts.paper.as_deref();
        let title = job.report.as_deref().unwrap_or("Yaseir Print Job");

        // 1. Direct TCP 9100 transport (tcp://host:port or raw://host:port)
        if job.printer.starts_with("tcp://") || job.printer.starts_with("raw://") {
            let (host, port, _) = Self::parse_host_port(&job.printer, 9100)?;
            // No local driver exists for a raw network printer: the
            // server-reported roll width (from the driver forms it
            // synced) is the media truth used to fit the output.
            let media_width_dots = opts
                .media_roll_width_mm
                .and_then(print_render::receipt_dots_for_width_mm);
            let raw_bytes = match kind.as_str() {
                "pdf" => Self::pdf_to_raw_stream(
                    &data,
                    &opts.mode,
                    paper,
                    media_width_dots,
                    opts.copies,
                    opts.cut,
                    None,
                )?,
                "image-png" => Self::png_to_raw_stream(
                    &data,
                    &opts.mode,
                    paper,
                    media_width_dots,
                    opts.copies,
                    None,
                )?,
                _ => {
                    let mut v = Vec::with_capacity(data.len() * opts.copies.max(1) as usize);
                    for _ in 0..opts.copies.max(1) {
                        v.extend_from_slice(&data);
                    }
                    v
                }
            };
            print_raw::tcp9100::send_raw(
                &host,
                port,
                &raw_bytes,
                Duration::from_secs(5),
                Duration::from_secs(20),
            )
            .map_err(|e| BackendError::ambiguous("TCP9100_ERROR", &e.to_string()))?;
            return Ok(());
        }

        // 2. Direct IPP transport (ipp://host:port/path or mode == "ipp")
        if job.printer.starts_with("ipp://") || opts.mode == "ipp" {
            // IPP submissions are PDF documents (the only format the
            // client negotiates). Anything else - a POS receipt image,
            // raw printer language - must never be sent labeled as
            // PDF; refuse loudly instead of corrupting the job.
            if kind != "pdf" {
                return Err(BackendError::fatal(
                    "IPP_REQUIRES_PDF",
                    "IPP printers accept PDF documents only; images and raw printer language cannot be sent over IPP",
                ));
            }
            let (host, port, path) = Self::parse_host_port(&job.printer, 631)?;
            let mut client = print_ipp::IppClient::new(&host, port);
            client.path = path.clone();
            client.printer_uri = format!("ipp://{host}:{port}{path}");
            let caps = client
                .get_printer_attributes()
                .map_err(|e| BackendError::retryable("IPP_ATTRIBUTES_ERROR", &e.to_string()))?;
            if caps.is_blocked() {
                return Err(BackendError::retryable(
                    "PRINTER_STOPPED",
                    &format!(
                        "IPP printer is blocked: {}",
                        caps.blocking_reasons().join(", ")
                    ),
                ));
            }
            let fmt = caps
                .choose_format()
                .map_err(|e| BackendError::fatal("IPP_FORMAT_UNSUPPORTED", &e.to_string()))?;
            client
                .print_job(&data, fmt, title, opts.copies.max(1))
                .map_err(|e| BackendError::ambiguous("IPP_PRINT_ERROR", &e.to_string()))?;
            return Ok(());
        }

        // 3. Windows Spooler / Local / Shared / Virtual printer
        let discovered = print_win::discover_printers().map_err(|e| {
            BackendError::retryable("SPOOLER_UNAVAILABLE", &format!("discover_printers: {e}"))
        })?;
        let mut target_kind: Option<PrinterKind> = None;
        let target_name = if let Some(matched) = resolve_printer(&job.printer, &discovered) {
            target_kind = Some(matched.kind);
            if needs_intervention(matched.status) {
                return Err(BackendError::retryable(
                    Self::status_code(matched.status),
                    &format!("printer {:?} status is {:?}", matched.name, matched.status),
                ));
            }
            matched.name.clone()
        } else if !discovered.is_empty() {
            return Err(BackendError::fatal(
                "PRINTER_NOT_FOUND",
                &format!("no installed printer matches pattern {:?}", job.printer),
            ));
        } else {
            job.printer.clone()
        };

        // Raw printer language can never go to a virtual PDF/XPS
        // writer: the spooler would write the raw bytes into a
        // document file, producing a corrupt, unopenable "PDF".
        if let Some(pk) = target_kind {
            if print_core::printer::raw_delivery_blocked(&kind, &opts.mode, pk) {
                return Err(BackendError::fatal(
                    "RAW_TO_VIRTUAL_PRINTER",
                    "raw printer language (ESC/POS / ZPL / TSPL / EPL) cannot be sent to a virtual PDF/XPS printer; choose a physical printer",
                ));
            }
        }

        if let Ok(live_status) = print_win::printer_status(&target_name) {
            if needs_intervention(live_status) {
                return Err(BackendError::retryable(
                    Self::status_code(live_status),
                    &format!("printer {target_name:?} status is {live_status:?}"),
                ));
            }
        }

        let target_width_dots =
            if opts.mode == "escpos" && matches!(kind.as_str(), "pdf" | "image-png") {
                Some(Self::receipt_width_dots(&target_name, paper)?)
            } else {
                None
            };

        // Label modes: the fit box is the printer's largest discrete
        // form (or its roll width when only continuous media exists),
        // so a rendered label can never exceed the physical media.
        let label_box = if matches!(opts.mode.as_str(), "zpl" | "tspl" | "epl")
            && matches!(kind.as_str(), "pdf" | "image-png")
        {
            print_win::printer_capabilities(&target_name)
                .ok()
                .and_then(|caps| {
                    let mut best: Option<(u32, u32)> = None;
                    let mut roll_w: Option<u32> = None;
                    for form in &caps.forms {
                        let w = (form.size.width_mm * 203.0 / 25.4) as u32;
                        if form.roll {
                            roll_w = Some(w);
                            continue;
                        }
                        let cand = (w, (form.size.height_mm * 203.0 / 25.4) as u32);
                        let area = cand.0 as u64 * cand.1 as u64;
                        if best.map_or(true, |b: (u32, u32)| area > b.0 as u64 * b.1 as u64) {
                            best = Some(cand);
                        }
                    }
                    best.or_else(|| roll_w.map(|w| (w, u32::MAX)))
                })
        } else {
            None
        };

        match (kind.as_str(), opts.mode.as_str()) {
            ("pdf", "escpos" | "zpl" | "tspl" | "epl") => {
                let raw_bytes = Self::pdf_to_raw_stream(
                    &data,
                    &opts.mode,
                    paper,
                    target_width_dots,
                    opts.copies,
                    opts.cut,
                    label_box,
                )?;
                print_win::print_raw(&target_name, title, &raw_bytes)
                    .map_err(|e| BackendError::ambiguous("SPOOLER_RAW_ERROR", &e.to_string()))?;
                Ok(())
            }
            ("pdf", "gdi") => {
                let policy = Self::parse_scale_policy(opts.scale_policy.as_deref());
                // Validate page geometry against the current Windows driver.
                let capabilities =
                    print_win::printer_capabilities(&target_name).map_err(|error| {
                        BackendError::fatal(
                            "PRINTER_CAPABILITIES_UNAVAILABLE",
                            &format!(
                                "The Windows driver did not expose media capabilities: {error}"
                            ),
                        )
                    })?;
                if let Ok(info) = print_render::parse_pdf_info(&data) {
                    for page in &info.pages {
                        if capabilities.forms.is_empty() && !capabilities.custom_size_supported {
                            return Err(BackendError::fatal(
                                "PRINTER_MEDIA_UNAVAILABLE",
                                "The installed driver reported no paper forms or custom-size limits; install/update the vendor printer driver.",
                            ));
                        }
                        let result = if policy == ScalePolicy::CustomAllowed {
                            match_paper_auto(page.size, &capabilities)
                        } else {
                            match_paper(page.size, &capabilities.forms, policy)
                        };
                        result
                            .map_err(|e| BackendError::fatal("PAPER_MISMATCH", &e.to_string()))?;
                    }
                }
                let render_mode = if opts.color {
                    RenderMode::Color
                } else {
                    RenderMode::Grayscale
                };
                let gdi_opts = print_win::GdiJobOptions {
                    mode: render_mode,
                    dpi: 300,
                    copies: opts.copies.max(1),
                    duplex: opts.duplex,
                    color: opts.color,
                    scale_policy: policy,
                    tray: opts.tray.clone(),
                };
                print_win::print_pdf_gdi_with_options(&target_name, title, &data, &gdi_opts)
                    .map_err(|e| BackendError::ambiguous("GDI_PRINT_ERROR", &e.to_string()))?;
                Ok(())
            }
            ("pdf", _) => Err(BackendError::fatal(
                "UNKNOWN_MODE",
                &format!("unsupported print mode {:?} for this document", opts.mode),
            )),
            ("image-png", "gdi") => {
                let canvas = print_render::decode_png(&data)
                    .map_err(|e| BackendError::fatal("IMAGE_DECODE_ERROR", &e.to_string()))?;
                print_win::print_canvas_gdi(&target_name, title, &canvas, opts.copies.max(1))
                    .map_err(|e| BackendError::ambiguous("GDI_PRINT_ERROR", &e.to_string()))?;
                Ok(())
            }
            ("image-png", "escpos" | "zpl" | "tspl" | "epl") => {
                let raw_bytes = Self::png_to_raw_stream(
                    &data,
                    &opts.mode,
                    paper,
                    target_width_dots,
                    opts.copies,
                    label_box,
                )?;
                print_win::print_raw(&target_name, title, &raw_bytes)
                    .map_err(|e| BackendError::ambiguous("SPOOLER_RAW_ERROR", &e.to_string()))?;
                Ok(())
            }
            ("image-png", _) => Err(BackendError::fatal(
                "UNKNOWN_MODE",
                &format!("unsupported print mode {:?} for this image", opts.mode),
            )),
            _ => {
                for _ in 0..opts.copies.max(1) {
                    print_win::print_raw(&target_name, title, &data).map_err(|e| {
                        BackendError::ambiguous("SPOOLER_RAW_ERROR", &e.to_string())
                    })?;
                }
                Ok(())
            }
        }
    }
}

/// Spawn the background worker thread that drains queued jobs across all
/// printer patterns using `print_core::Worker`.
pub fn spawn_worker_loop(db_path: String, wake: Arc<(Mutex<bool>, Condvar)>) {
    std::thread::spawn(move || {
        let mut entropy: u64 = 0x5EED_2026;
        loop {
            let mut did_work = false;
            if let Ok(mut worker_queue) = Queue::open(&db_path) {
                // Fail jobs wedged in a working state by a dead or
                // stuck worker (RECONCILED_STALE) instead of letting
                // them hang forever; fresh jobs are never touched.
                let _ = worker_queue.reconcile_stale();
                let printers = worker_queue.queued_printers().unwrap_or_default();
                for printer in printers {
                    entropy = entropy.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let mut worker = Worker {
                        queue: &mut worker_queue,
                        backend: NativePrintBackend {
                            db_path: db_path.clone(),
                        },
                        cfg: WorkerConfig::default(),
                        sleep: |ms| std::thread::sleep(Duration::from_millis(ms)),
                        entropy,
                    };
                    if let Ok(outcome) = worker.run_once(&printer) {
                        if outcome != Outcome::Idle {
                            did_work = true;
                        }
                    }
                }
            }
            if !did_work {
                let (lock, cvar) = &*wake;
                if let Ok(mut pending) = lock.lock() {
                    if !*pending {
                        let _ = cvar.wait_timeout(pending, Duration::from_secs(2));
                    } else {
                        *pending = false;
                    }
                } else {
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base64_and_host_port_parsing() {
        let decoded = decode_base64("JVBERi0xLjQ=").unwrap();
        assert_eq!(decoded, b"%PDF-1.4");
        let (h, p, path) =
            NativePrintBackend::parse_host_port("tcp://192.168.1.200:9100", 9100).unwrap();
        assert_eq!(h, "192.168.1.200");
        assert_eq!(p, 9100);
        assert_eq!(path, "/ipp/print");
        let (h6, p6, path6) =
            NativePrintBackend::parse_host_port("ipp://[::1]:631/ipp/print", 631).unwrap();
        assert_eq!(h6, "::1");
        assert_eq!(p6, 631);
        assert_eq!(path6, "/ipp/print");
    }
}

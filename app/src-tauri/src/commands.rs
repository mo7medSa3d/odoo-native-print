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
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::{Manager, State};

/// Lock the shared queue, recovering from mutex poisoning. A panic
/// inside print-core while the lock was held used to turn every later
/// command into `queue_poisoned` until restart (audit M3) — printing
/// dead for the session from one fault. The queue's data lives in
/// transactional SQLite, so the recovered guard is safe to keep
/// using; the poison event is logged to stderr.
fn lock_queue(queue: &Mutex<Queue>) -> std::sync::MutexGuard<'_, Queue> {
    match queue.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("Yaseir Print: queue mutex was poisoned; recovering the guard");
            poisoned.into_inner()
        }
    }
}

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
    /// Tab registry for the tabbed browser (owner brief,
    /// 2026-11-05): one window, a local tab bar, and one child
    /// webview per tab, all sharing one data directory so Odoo
    /// sessions and cookies behave as in a single browser profile.
    pub tabs: RwLock<TabsState>,
}

/// One tab in the tabbed browser. The tab bar renders from these
/// snapshots; titles are derived from URLs because a child webview
/// does not expose its document title to Rust.
#[derive(Debug, Clone, Serialize)]
pub struct TabInfo {
    pub label: String,
    pub url: String,
    pub title: String,
    pub is_pos: bool,
    pub is_local: bool,
    pub active: bool,
}

#[derive(Debug, Default)]
pub struct TabsState {
    pub order: Vec<String>,
    pub active: Option<String>,
    pub next_id: u64,
    pub urls: std::collections::HashMap<String, String>,
}

impl TabsState {
    pub fn register(&mut self, label: &str, url: &str) {
        if !self.order.iter().any(|l| l == label) {
            self.order.push(label.to_string());
        }
        self.urls.insert(label.to_string(), url.to_string());
        if self.active.is_none() {
            self.active = Some(label.to_string());
        }
    }

    pub fn set_url(&mut self, label: &str, url: &str) {
        self.urls.insert(label.to_string(), url.to_string());
    }

    pub fn url_of(&self, label: &str) -> Option<&str> {
        self.urls.get(label).map(|s| s.as_str())
    }

    pub fn is_pos_tab(&self, label: &str) -> bool {
        self.url_of(label).map(is_pos_url).unwrap_or(false)
    }

    pub fn snapshot(&self) -> Vec<TabInfo> {
        self.order
            .iter()
            .map(|label| {
                let url = self.url_of(label).unwrap_or("").to_string();
                let parsed = tauri::Url::parse(&url).ok();
                TabInfo {
                    label: label.clone(),
                    title: tab_title_for(&url),
                    is_pos: is_pos_url(&url),
                    is_local: parsed.as_ref().map(is_local_app_url).unwrap_or(false),
                    active: self.active.as_deref() == Some(label.as_str()),
                    url,
                }
            })
            .collect()
    }
}

/// Point of Sale URLs live under /pos (the POS UI is /pos/ui).
pub fn is_pos_url(url: &str) -> bool {
    tauri::Url::parse(url)
        .map(|u| {
            let path = u.path();
            path == "/pos" || path.starts_with("/pos/")
        })
        .unwrap_or(false)
}

/// Human tab title for a URL. Deliberately deterministic (unit
/// tested): the tab bar is chrome, not content, so titles come
/// from the route, never from page-supplied strings.
pub fn tab_title_for(url: &str) -> String {
    let Ok(u) = tauri::Url::parse(url) else {
        return "Odoo".to_string();
    };
    if is_local_app_url(&u) {
        if url.contains("index.html") {
            return "Control Center".to_string();
        }
        return "Yaseir Print".to_string();
    }
    let path = u.path();
    if is_pos_url(url) {
        return "Point of Sale".to_string();
    }
    if path.starts_with("/web/login") {
        return "Sign in".to_string();
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.first() == Some(&"odoo") && segments.len() > 1 {
        let app = segments[1].replace('-', " ");
        let mut chars = app.chars();
        return match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => "Odoo".to_string(),
        };
    }
    "Odoo".to_string()
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
    /// Label gap in millimetres (TSPL GAP / EPL Q gap). Absent =
    /// the printer's 3 mm default. Only label modes use it.
    #[serde(default)]
    pub label_gap_mm: Option<f64>,
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
        label_gap_mm: req.label_gap_mm,
        source_width_mm: None,
    };
    let id = {
        let mut queue = lock_queue(&state.queue);
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
    // Same key validation as print_pdf/print_image (audit M1): an
    // unvalidated key becomes the job id verbatim, so an empty or
    // colliding key would silently dedupe two different documents
    // into one job.
    if !print_core::security::valid_idempotency_key(&req.idempotency_key) {
        return Err("bad_idempotency_key".to_string());
    }
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
        label_gap_mm: None,
        source_width_mm: None,
    };
    let id = {
        let mut queue = lock_queue(&state.queue);
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
    pub label_gap_mm: Option<f64>,
    /// Physical width (mm) of the source element the image was
    /// rendered from (a POS receipt's on-screen width). Last-resort
    /// width evidence for receipt rendering; sanitized at enqueue.
    #[serde(default)]
    pub source_width_mm: Option<f64>,
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
    // Width evidence from the page is a hint, not truth: keep only
    // plausible physical widths (a receipt wider than half a metre
    // or narrower than 20 mm is a bug or an attack, not media).
    let source_width_mm = req
        .source_width_mm
        .filter(|w| w.is_finite() && (20.0..=500.0).contains(w));
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
        label_gap_mm: req.label_gap_mm,
        source_width_mm,
    };
    let id = {
        let mut queue = lock_queue(&state.queue);
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
    let queue = lock_queue(&state.queue);
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
    let queue = lock_queue(&state.queue);
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
        let queue = lock_queue(&state.queue);
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

/// Boot-page reachability probe: TCP-connect to the configured
/// origin. Host/port come from print-core's security parser (the
/// one parser for this value — audit L5), and every resolved
/// address is tried under one shared 3 s deadline: a dual-stack
/// server whose first family is dead must not read as "down" when
/// the webview itself would connect via the other family.
#[tauri::command]
pub fn probe_origin(webview: tauri::Webview, state: State<'_, AppState>) -> Result<bool, String> {
    check_local_only(&webview)?;
    let origin = match state.configured_origin.read() {
        Ok(guard) => guard.clone(),
        Err(_) => None,
    };
    let Some(origin) = origin else {
        return Ok(false);
    };
    let Some((host, port)) = print_core::security::origin_host_port(&origin) else {
        return Ok(false);
    };
    let addrs = match (host.as_str(), port).to_socket_addrs() {
        Ok(a) => a,
        Err(_) => return Ok(false),
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    for sock in addrs {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        if std::net::TcpStream::connect_timeout(&sock, remaining).is_ok() {
            return Ok(true);
        }
    }
    Ok(false)
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
    // Test prints are a Control Center feature: the Odoo page must
    // never fire raw calibrations at local printers (audit B1).
    check_local_only(&webview)?;
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
        // Fail closed: a raw calibration is refused whenever the
        // target's kind cannot be positively verified as physical.
        let discovered = print_win::discover_printers().unwrap_or_default();
        let found = discovered.iter().find(|p| p.name == printer.trim());
        let verified_physical = matches!(found, Some(f) if f.kind != PrinterKind::Virtual);
        if !verified_physical {
            return Err(
                "This calibration sends raw printer language (ESC/POS / ZPL / TSPL). It was refused because the target printer could not be verified as a physical printer (a virtual PDF/XPS writer would turn it into a corrupt file). Choose a physical printer from the discovered list."
                    .to_string(),
            );
        }
        // The language must be the one this printer actually
        // speaks (owner, 2026-11-04): an ESC/POS calibration to a
        // ZPL label printer, or a ZPL label to an office printer,
        // is refused up front instead of printing garbage. The
        // Control Center never offers the mismatch; this is the
        // same rule enforced at the command for stale pages.
        if let Some(f) = found {
            let wanted = match kind.as_str() {
                "thermal-80mm" | "thermal-58mm" | "escpos" => "escpos",
                "zpl-label" | "zpl" => "zpl",
                "tspl-label" | "tspl" => "tspl",
                _ => "",
            };
            if !wanted.is_empty() && f.protocol != wanted {
                return Err(format!(
                    "This calibration sends {} printer language, but \"{}\" is a {} printer ({}). It was refused so the printer never receives a language it cannot print. Choose the calibration that matches this printer.",
                    wanted.to_uppercase(),
                    f.name,
                    f.printer_class,
                    f.protocol.to_uppercase()
                ));
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
                    .map(|p| print_render::receipt_dots_for_paper(p) == Some(384))
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
                    label_gap_mm: None,
                    source_width_mm: None,
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
                    label_gap_mm: None,
                    source_width_mm: None,
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
                    label_gap_mm: None,
                    source_width_mm: None,
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
                    label_gap_mm: None,
                    source_width_mm: None,
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
        let mut queue = lock_queue(&state.queue);
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
        let mut queue = lock_queue(&state.queue);
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
    let mut queue = lock_queue(&state.queue);
    queue.cancel_job(&job_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_finished_cmd(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    check_origin(&webview, &state)?;
    check_local_only(&webview)?;
    let mut queue = lock_queue(&state.queue);
    queue.clear_finished().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_diagnostics(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<String, String> {
    check_origin(&webview, &state)?;
    check_local_only(&webview)?;
    let queue = lock_queue(&state.queue);
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

/// Create a tab for `url` and make it active. Shared by the
/// tab_open command, the POS navigation rule and window.open
/// handling. Async: creating a webview from a synchronous command
/// can deadlock on Windows.
pub async fn open_tab_internal(
    app: &tauri::AppHandle,
    state: &AppState,
    url: tauri::Url,
) -> Result<String, String> {
    let label = {
        let mut tabs = state
            .tabs
            .write()
            .map_err(|_| "tabs_lock_poisoned".to_string())?;
        tabs.next_id += 1;
        let label = format!("tab-{}", tabs.next_id);
        tabs.register(&label, url.as_str());
        tabs.active = Some(label.clone());
        label
    };
    let web_url = if is_local_app_url(&url) {
        let path = url.path().trim_start_matches('/');
        tauri::WebviewUrl::App(path.into())
    } else {
        tauri::WebviewUrl::External(url.clone())
    };
    crate::create_tab_webview(app, &label, web_url)?;
    crate::activate_tab(app, &label);
    Ok(label)
}

/// Open a URL in a new tab. Callable from the configured Odoo
/// origin (allow-app-shell): the URL must belong to that origin —
/// validated here in Rust, never trusted from the page.
#[tauri::command]
pub async fn tab_open(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> Result<String, String> {
    // An empty URL is the tab bar's "new tab" action: a fresh tab
    // on the configured origin (the Control Center when none is
    // configured yet).
    if url.trim().is_empty() {
        let origin = state
            .configured_origin
            .read()
            .map_err(|_| "config_lock_poisoned".to_string())?
            .clone();
        let target = match origin {
            Some(origin) => origin,
            None => {
                if cfg!(windows) {
                    "http://tauri.localhost/index.html".to_string()
                } else {
                    "tauri://localhost/index.html".to_string()
                }
            }
        };
        let parsed = tauri::Url::parse(&target).map_err(|_| "bad_tab_url".to_string())?;
        return open_tab_internal(&app, &state, parsed).await;
    }
    let parsed = tauri::Url::parse(&url).map_err(|_| "bad_tab_url".to_string())?;
    if !is_local_app_url(&parsed) {
        let configured = state
            .configured_origin
            .read()
            .map_err(|_| "config_lock_poisoned".to_string())?
            .clone()
            .ok_or_else(|| "no_origin_configured".to_string())?;
        let origin = parsed.origin().ascii_serialization();
        if !origin_is_allowed(&configured, &origin) {
            return Err("tab_url_not_on_configured_origin".to_string());
        }
    }
    open_tab_internal(&app, &state, parsed).await
}

/// Tab list for the tab bar (local pages only: the tab bar itself).
#[tauri::command]
pub fn tab_list(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<Vec<TabInfo>, String> {
    check_local_only(&webview)?;
    let tabs = state
        .tabs
        .read()
        .map_err(|_| "tabs_lock_poisoned".to_string())?;
    Ok(tabs.snapshot())
}

/// Activate a tab (tab bar only).
#[tauri::command]
pub fn tab_switch(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    label: String,
) -> Result<(), String> {
    check_local_only(&webview)?;
    {
        let tabs = state
            .tabs
            .read()
            .map_err(|_| "tabs_lock_poisoned".to_string())?;
        if !tabs.order.iter().any(|l| l == &label) {
            return Err("unknown_tab".to_string());
        }
    }
    crate::activate_tab(&app, &label);
    Ok(())
}

/// Close a tab (tab bar only). The window never drops to zero
/// tabs: closing the last one opens a fresh tab on the configured
/// origin (or the Control Center when no origin is configured).
#[tauri::command]
pub async fn tab_close(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    label: String,
) -> Result<(), String> {
    check_local_only(&webview)?;
    let (was_active, neighbor) = {
        let mut tabs = state
            .tabs
            .write()
            .map_err(|_| "tabs_lock_poisoned".to_string())?;
        let idx = tabs.order.iter().position(|l| l == &label);
        let Some(idx) = idx else {
            return Err("unknown_tab".to_string());
        };
        let was_active = tabs.active.as_deref() == Some(label.as_str());
        tabs.order.remove(idx);
        tabs.urls.remove(&label);
        let neighbor = if tabs.order.is_empty() {
            None
        } else {
            Some(tabs.order[idx.min(tabs.order.len() - 1)].clone())
        };
        if was_active {
            tabs.active = neighbor.clone();
        }
        (was_active, neighbor)
    };
    if let Some(wv) = app.get_webview(&label) {
        let _ = wv.close();
    }
    if was_active {
        match neighbor {
            Some(next) => crate::activate_tab(&app, &next),
            None => {
                let origin = state
                    .configured_origin
                    .read()
                    .map_err(|_| "config_lock_poisoned".to_string())?
                    .clone();
                match origin {
                    Some(origin) => {
                        let url =
                            tauri::Url::parse(&origin).map_err(|_| "bad_origin_url".to_string())?;
                        open_tab_internal(&app, &state, url).await?;
                    }
                    None => {
                        let url = tauri::Url::parse("tauri://localhost/index.html")
                            .map_err(|_| "bad_local_url".to_string())?;
                        open_tab_internal(&app, &state, url).await?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// The active content webview, for the navigation commands.
fn active_tab_webview(app: &tauri::AppHandle, state: &AppState) -> Result<tauri::Webview, String> {
    let label = {
        let tabs = state
            .tabs
            .read()
            .map_err(|_| "tabs_lock_poisoned".to_string())?;
        tabs.active.clone()
    }
    .ok_or_else(|| "no_active_tab".to_string())?;
    app.get_webview(&label)
        .ok_or_else(|| "active_tab_missing".to_string())
}

/// Browser-style navigation for the tab bar: history back on the
/// ACTIVE tab's webview. Odoo is an SPA whose screens stack routes
/// with no visible way back; these give the user the browser
/// controls back.
#[tauri::command]
pub fn nav_back(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_local_only(&webview)?;
    let target = active_tab_webview(&app, &state)?;
    target.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
pub fn nav_forward(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_local_only(&webview)?;
    let target = active_tab_webview(&app, &state)?;
    target.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
pub fn nav_reload(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_local_only(&webview)?;
    let target = active_tab_webview(&app, &state)?;
    target.reload().map_err(|e| e.to_string())
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
///
/// Control Center only (audit M2): the wipe logs the user out, so
/// the capability grant excludes remote pages AND this command
/// re-checks local-only in Rust — a script on the Odoo origin must
/// never be able to loop it into a permanent forced logout. The
/// boot watchdog card points users here instead of invoking it.
#[tauri::command]
pub async fn clear_web_data_cmd(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_local_only(&webview)?;
    // Tabbed browser: every tab shares one data directory, so the
    // wipe is applied per webview (any one clears the shared
    // profile) and every remote tab is sent back to the origin to
    // sign in again — the same clean slate the single-webview
    // build produced.
    let labels = {
        let tabs = state
            .tabs
            .read()
            .map_err(|_| "tabs_lock_poisoned".to_string())?;
        tabs.order.clone()
    };
    for label in &labels {
        if let Some(wv) = app.get_webview(label) {
            wv.clear_all_browsing_data().map_err(|e| e.to_string())?;
        }
    }
    let origin = state
        .configured_origin
        .read()
        .map_err(|_| "config_lock_poisoned".to_string())?
        .clone();
    if let Some(origin) = origin {
        let url = origin.parse::<tauri::Url>().map_err(|e| e.to_string())?;
        let snapshot = {
            let tabs = state
                .tabs
                .read()
                .map_err(|_| "tabs_lock_poisoned".to_string())?;
            tabs.snapshot()
        };
        for tab in snapshot.iter().filter(|t| !t.is_local) {
            if let Some(wv) = app.get_webview(&tab.label) {
                let _ = wv.navigate(url.clone());
            }
        }
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

/// Per-printer defaults for the Control Center (owner brief,
/// 2026-11-05). Local pages only: a remote Odoo page must never
/// read or rewrite this PC's printer configuration.
#[tauri::command]
pub fn get_printer_settings(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<config::PrinterSettingsMap, String> {
    check_local_only(&webview)?;
    Ok(config::load_printer_settings(&state.config_dir))
}

#[tauri::command]
pub fn set_printer_settings(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    printer: String,
    settings: config::PrinterSettings,
) -> Result<config::PrinterSettingsMap, String> {
    check_local_only(&webview)?;
    let name = printer.trim().to_string();
    if name.is_empty() || name.chars().count() > 256 {
        return Err("invalid printer name".to_string());
    }
    for v in [&settings.paper, &settings.tray].into_iter().flatten() {
        if v.chars().count() > 160 {
            return Err("printer setting value too long".to_string());
        }
    }
    let mut map = config::load_printer_settings(&state.config_dir);
    if settings == config::PrinterSettings::default() {
        map.remove(&name);
    } else {
        map.insert(name, settings);
    }
    config::save_printer_settings(&state.config_dir, &map)?;
    Ok(map)
}

#[tauri::command]
pub async fn open_control_center(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_origin(&webview, &state)?;
    // The Control Center lives in its own tab (owner brief,
    // 2026-11-05): opening it never replaces the Odoo page the
    // user was working on.
    let existing = {
        let tabs = state
            .tabs
            .read()
            .map_err(|_| "tabs_lock_poisoned".to_string())?;
        tabs.order
            .iter()
            .find(|label| {
                tabs.url_of(label)
                    .and_then(|u| tauri::Url::parse(u).ok())
                    .map(|u| is_local_app_url(&u) && u.path().ends_with("index.html"))
                    .unwrap_or(false)
            })
            .cloned()
    };
    if let Some(label) = existing {
        crate::activate_tab(&app, &label);
        return Ok(());
    }
    let local_url: tauri::Url = if cfg!(windows) {
        "http://tauri.localhost/index.html"
            .parse()
            .map_err(|_| "bad_local_url".to_string())?
    } else {
        "tauri://localhost/index.html"
            .parse()
            .map_err(|_| "bad_local_url".to_string())?
    };
    open_tab_internal(&app, &state, local_url).await?;
    Ok(())
}

#[tauri::command]
pub async fn open_odoo_origin(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_origin(&webview, &state)?;
    let origin = state
        .configured_origin
        .read()
        .map_err(|_| "config_lock_poisoned".to_string())?
        .clone()
        .ok_or_else(|| "no_origin_configured".to_string())?;
    let url: tauri::Url = origin.parse().map_err(|_| "bad_origin_url".to_string())?;
    // Tabbed browser: prefer an existing remote tab (switch back
    // to Odoo without losing anything), navigate the calling tab
    // when it is the only candidate (the boot page flow), and give
    // the tab bar a fresh tab.
    let caller = webview.label().to_string();
    let (existing, caller_is_tab) = {
        let tabs = state
            .tabs
            .read()
            .map_err(|_| "tabs_lock_poisoned".to_string())?;
        let snapshot = tabs.snapshot();
        let existing = snapshot
            .iter()
            .find(|t| !t.is_local && t.label != caller)
            .map(|t| t.label.clone());
        let caller_is_tab = tabs.order.iter().any(|l| l == &caller);
        (existing, caller_is_tab)
    };
    if let Some(label) = existing {
        crate::activate_tab(&app, &label);
        if let Some(wv) = app.get_webview(&label) {
            wv.navigate(url).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if caller_is_tab {
        webview.navigate(url).map_err(|e| e.to_string())?;
        return Ok(());
    }
    open_tab_internal(&app, &state, url).await?;
    Ok(())
}

// --- Background Print Worker & Hardware Dispatch ----------------------

/// Parameters shared by the PDF and PNG raw-render paths.
/// `render_dpi` is the device render resolution
/// (driver-reported; 203 when unknown): label bitmaps are
/// emitted in device dots, so rendering at any other DPI
/// would mis-scale the label.
#[derive(Clone, Copy)]
struct RawRender<'a> {
    mode: &'a str,
    paper: Option<&'a str>,
    target_width_dots: Option<u32>,
    copies: u16,
    label_box: Option<(u32, u32)>,
    render_dpi: u32,
    label_gap_mm: Option<f64>,
}

struct NativePrintBackend {
    db_path: String,
    config_dir: std::path::PathBuf,
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

    /// Scale a 203-dpi print-head profile width (see
    /// `receipt_dots_for_paper`) to the device's reported render
    /// resolution. A 300-dpi head prints each ESC/POS dot smaller,
    /// so the unscaled dot count would come out physically narrow;
    /// the driver-reported resolution is the best evidence
    /// available. With no reported resolutions this is the
    /// identity (203 dpi).
    fn scale_head_dots(dots: u32, caps: &print_core::printer::PrinterCapabilities) -> u32 {
        let dpi = print_core::printer::preferred_raw_render_dpi(&caps.resolutions_dpi);
        ((dots as u64 * dpi as u64 + 101) / 203) as u32
    }

    /// Resolve the ESC/POS raster width (dots) for a receipt job
    /// from the best width evidence available, in order:
    /// 1. an explicit paper choice on the job (verified against
    ///    the driver's roll forms when it reports any);
    /// 2. the roll width the page resolved from this printer's
    ///    driver capabilities at click time (rides on the job);
    /// 3. the driver's own forms read here: a roll form first,
    ///    else the narrowest receipt-range form — a POS queue
    ///    whose driver reports its roll as a plain sheet form
    ///    ("80 x 297 mm") still yields its real width;
    /// 4. the receipt's own physical width (its rendered width,
    ///    sent by the page).
    ///
    /// When both (3) and (4) exist but disagree wildly, the
    /// receipt's own width wins: the form list then plainly does
    /// not describe receipt media. Only a job with NO width
    /// evidence at all fails — printing at an invented width is
    /// worse than a clear error, but a valid printer is never
    /// refused for lacking one specific form flag (owner
    /// directive, 2026-11-05: POS prints to any printer, rendered
    /// to that printer's dimensions).
    fn receipt_width_dots(
        printer: &str,
        paper_override: Option<&str>,
        job_media_mm: Option<f64>,
        source_width_mm: Option<f64>,
    ) -> Result<u32, BackendError> {
        if let Some(paper) = paper_override {
            let dots = print_render::receipt_dots_for_paper(paper).ok_or_else(|| {
                BackendError::fatal(
                    "AUTO_MEDIA_UNAVAILABLE",
                    &format!("unknown receipt paper {paper:?}; cannot derive a print width"),
                )
            })?;
            // An explicit paper choice is verified against the driver's
            // reported roll forms when they are available: rendering an
            // 80mm receipt for a 58mm roll (or the reverse) clips or
            // scales the customer's receipt. A FAILED capabilities read
            // must not kill it (2026-11-05 practicality round): the
            // paper was chosen deliberately, so print it unverified —
            // the same rule step (2) applies to job-carried widths.
            let caps = match print_win::printer_capabilities(printer) {
                Ok(caps) => caps,
                Err(_) => return Ok(dots),
            };
            {
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
            return Ok(Self::scale_head_dots(dots, &caps));
        }
        let sane = |w: &f64| w.is_finite() && (20.0..=500.0).contains(w);
        // (2) The job's own media width: page-side driver evidence
        // captured at click time. Head-DPI scaling still applies
        // when the driver can be read here; a failed read must not
        // kill a job whose width is already known.
        if let Some(dots) = job_media_mm
            .filter(sane)
            .and_then(print_render::receipt_dots_for_width_mm)
        {
            return Ok(match print_win::printer_capabilities(printer) {
                Ok(caps) => Self::scale_head_dots(dots, &caps),
                Err(_) => dots,
            });
        }
        // (3) Driver forms read here: roll first, else the
        // narrowest receipt-range form.
        let caps = print_win::printer_capabilities(printer).ok();
        let form_mm = caps.as_ref().and_then(|caps| {
            caps.forms
                .iter()
                .find(|form| form.roll)
                .map(|form| form.size.width_mm)
                .or_else(|| {
                    caps.forms
                        .iter()
                        .map(|form| form.size.width_mm)
                        .filter(|w| (40.0..=220.0).contains(w))
                        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                })
        });
        let source_mm = source_width_mm.filter(sane);
        // (3) vs (4): a form list that disagrees with the receipt
        // itself by more than 30 mm is not describing this media.
        if let Some(dots) = Self::form_width_dots(form_mm, source_mm) {
            if let Some(caps) = &caps {
                return Ok(Self::scale_head_dots(dots, caps));
            }
            return Ok(dots);
        }
        // (4) The receipt's own physical width.
        if let Some(dots) = source_mm.and_then(print_render::receipt_dots_for_width_mm) {
            return Ok(dots);
        }
        if caps.is_none() {
            // A capabilities read can fail transiently (spooler
            // busy, driver still initializing): retryable, so the
            // job rides the backoff envelope instead of dying on a
            // momentary read failure.
            return Err(BackendError::retryable(
                "PRINTER_CAPABILITIES_UNAVAILABLE",
                "The Windows driver did not expose printer capabilities, and the job carries no media width.",
            ));
        }
        Err(BackendError::fatal(
            "AUTO_MEDIA_UNAVAILABLE",
            "No width evidence for ESC/POS rendering: the driver reports no usable form and the job carries no media width.",
        ))
    }

    /// The (3)-vs-(4) width choice, pure for testing: the
    /// form-derived width converts to dots unless the receipt's
    /// own width disagrees with it by more than 30 mm, in which
    /// case the form evidence is discarded (the caller then uses
    /// the receipt's width).
    fn form_width_dots(form_mm: Option<f64>, source_mm: Option<f64>) -> Option<u32> {
        match (form_mm, source_mm) {
            (Some(f), Some(s)) if (f - s).abs() > 30.0 => None,
            (Some(f), _) => print_render::receipt_dots_for_width_mm(f),
            _ => None,
        }
    }

    fn parse_scale_policy(s: Option<&str>) -> ScalePolicy {
        match s.unwrap_or("custom_allowed") {
            "strict" => ScalePolicy::Strict,
            "nearest_larger_centered" => ScalePolicy::NearestLargerCentered,
            "nearest_larger_fit" => ScalePolicy::NearestLargerFit,
            _ => ScalePolicy::CustomAllowed,
        }
    }

    /// Printer-URI for the IPP client: IPv6 hosts must be
    /// bracketed (RFC 3986). `parse_host_port` strips the
    /// brackets, so re-adding them here is what makes
    /// `ipp://[::1]:631/...` round-trip. `IppClient::new`
    /// brackets the host itself for the HTTP connection; this
    /// URI is the `printer-uri` attribute sent to the printer.
    fn ipp_uri(host: &str, port: u16, path: &str) -> String {
        if host.contains(':') {
            format!("ipp://[{host}]:{port}{path}")
        } else {
            format!("ipp://{host}:{port}{path}")
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

    /// Configured label gap in millimetres; the printer's 3 mm
    /// default when the job carries none. Negative gaps are
    /// clamped to 0 (continuous media).
    fn label_gap_mm(gap: Option<f64>) -> f64 {
        gap.unwrap_or(3.0).max(0.0)
    }

    /// Configured label gap in dots at `dpi` (EPL `Q` parameter).
    fn label_gap_dots(gap: Option<f64>, dpi: u32) -> u32 {
        (Self::label_gap_mm(gap) * dpi as f64 / 25.4).round() as u32
    }

    fn pdf_to_raw_stream(
        pdf_bytes: &[u8],
        render: RawRender<'_>,
        cut: bool,
    ) -> Result<Vec<u8>, BackendError> {
        let RawRender {
            mode,
            paper,
            target_width_dots,
            copies,
            label_box,
            render_dpi,
            label_gap_mm,
        } = render;
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
                    .or_else(|| paper.and_then(print_render::receipt_dots_for_paper))
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
                            .render_page(i, RenderMode::Grayscale, render_dpi)
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
                        .render_page(i, RenderMode::Threshold, render_dpi)
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
                        .render_page(i, RenderMode::Threshold, render_dpi)
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
                    let tspl = print_raw::tspl::TsplLabel::new(
                        w_mm,
                        h_mm,
                        Self::label_gap_mm(label_gap_mm).round() as u32,
                    )
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
                        .render_page(i, RenderMode::Threshold, render_dpi)
                        .map_err(|e| BackendError::fatal("RENDER_FAILED", &e.to_string()))?;
                    let w_aligned = page.width_px.div_ceil(8) * 8;
                    let bm = MonoBitmap::from_packed(w_aligned, page.height_px, page.pixels)
                        .map_err(|e| BackendError::fatal("BITMAP_PACK_ERROR", &e.to_string()))?;
                    let bm = match label_box {
                        Some((w, h)) => print_render::bitmap::fit_mono_to_box(&bm, w, h),
                        None => bm,
                    };
                    out.extend_from_slice(&print_raw::epl::epl_label_copies_with_gap(
                        &bm,
                        copies,
                        Self::label_gap_dots(label_gap_mm, render_dpi),
                    ));
                }
                Ok(out)
            }
            _ => Err(BackendError::fatal(
                "UNKNOWN_MODE",
                &format!("unsupported print mode {mode:?} for a PDF document"),
            )),
        }
    }

    fn png_to_raw_stream(png_bytes: &[u8], render: RawRender<'_>) -> Result<Vec<u8>, BackendError> {
        // A PNG carries no intrinsic DPI; `render_dpi` is the
        // interpretation used for label sizing (203 when no
        // driver capabilities are known).
        let RawRender {
            mode,
            paper,
            target_width_dots,
            copies,
            label_box,
            render_dpi,
            label_gap_mm,
        } = render;
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
                let w_mm = ((mono.width_dots as f64) * 25.4 / render_dpi as f64)
                    .round()
                    .max(20.0) as u32;
                let h_mm = ((mono.height_dots as f64) * 25.4 / render_dpi as f64)
                    .round()
                    .max(20.0) as u32;
                Ok(print_raw::tspl::TsplLabel::new(
                    w_mm,
                    h_mm,
                    Self::label_gap_mm(label_gap_mm).round() as u32,
                )
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
                Ok(print_raw::epl::epl_label_copies_with_gap(
                    &mono,
                    copies,
                    Self::label_gap_dots(label_gap_mm, render_dpi),
                ))
            }
            "escpos" => {
                let target_width_dots = target_width_dots
                    .or_else(|| paper.and_then(print_render::receipt_dots_for_paper))
                    .ok_or_else(|| {
                        BackendError::fatal(
                            "AUTO_MEDIA_UNAVAILABLE",
                            "No width evidence is available for ESC/POS rendering (no driver form, no job media width, no source width).",
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
        let mut opts = q.options(&job.id).map_err(|e| {
            BackendError::retryable(
                "QUEUE_DB_ERROR",
                &format!("stored job options unreadable: {e}"),
            )
        })?;
        // Per-printer defaults from the Control Center fill whatever
        // this job did not specify (owner brief, 2026-11-05): the
        // user configures each printer once; a document carrying
        // its own paper/tray always wins.
        if let Some(defaults) = config::load_printer_settings(&self.config_dir).get(&job.printer) {
            if opts.paper.is_none() {
                opts.paper = defaults.paper.clone();
            }
            if opts.tray.is_none() {
                opts.tray = defaults.tray.clone();
            }
            if let Some(d) = defaults.duplex {
                opts.duplex = d;
            }
            if let Some(c) = defaults.color {
                opts.color = c;
            }
        }
        let paper = opts.paper.as_deref();
        let title = job.report.as_deref().unwrap_or("Yaseir Print Job");

        // 1. Direct TCP 9100 transport (tcp://host:port or raw://host:port)
        if job.printer.starts_with("tcp://") || job.printer.starts_with("raw://") {
            let (host, port, _) = Self::parse_host_port(&job.printer, 9100)?;
            // No local driver exists for a raw network printer:
            // the job's media width (resolved page-side from the
            // printer's synced driver forms) is the media truth;
            // the receipt's own physical width is the fallback —
            // a network POS printer prints at its real width from
            // either evidence, never neither (2026-11-05).
            let media_width_dots = opts
                .media_roll_width_mm
                .and_then(print_render::receipt_dots_for_width_mm)
                .or_else(|| {
                    opts.source_width_mm
                        .and_then(print_render::receipt_dots_for_width_mm)
                });
            let raw_bytes = match kind.as_str() {
                "pdf" => Self::pdf_to_raw_stream(
                    &data,
                    RawRender {
                        mode: &opts.mode,
                        paper,
                        target_width_dots: media_width_dots,
                        copies: opts.copies,
                        label_box: None,
                        render_dpi: 203,
                        label_gap_mm: opts.label_gap_mm,
                    },
                    opts.cut,
                )?,
                "image-png" => Self::png_to_raw_stream(
                    &data,
                    RawRender {
                        mode: &opts.mode,
                        paper,
                        target_width_dots: media_width_dots,
                        copies: opts.copies,
                        label_box: None,
                        render_dpi: 203,
                        label_gap_mm: opts.label_gap_mm,
                    },
                )?,
                _ => {
                    let mut v = Vec::with_capacity(data.len() * opts.copies.max(1) as usize);
                    for _ in 0..opts.copies.max(1) {
                        v.extend_from_slice(&data);
                    }
                    v
                }
            };
            // A connect-phase failure proves zero bytes reached
            // the printer (it is off, asleep, or unreachable), so
            // the job retries safely; only a write-phase failure is
            // ambiguous (2026-11-05 practicality round).
            print_raw::tcp9100::send_raw_phased(
                &host,
                port,
                &raw_bytes,
                Duration::from_secs(5),
                Duration::from_secs(20),
            )
            .map_err(|e| match e {
                print_raw::tcp9100::SendPhaseError::Connect(io) => {
                    BackendError::retryable("TCP9100_CONNECT_ERROR", &io.to_string())
                }
                print_raw::tcp9100::SendPhaseError::Write(io) => {
                    BackendError::ambiguous("TCP9100_ERROR", &io.to_string())
                }
            })?;
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
            client.printer_uri = Self::ipp_uri(&host, port, &path);
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
            // Honour the job's finishing options over IPP, validated
            // against the printer's reported capabilities: anything
            // requested that IPP cannot honour fails loudly instead
            // of being silently dropped.
            let mut ipp_options = print_ipp::IppJobOptions::default();
            if let Some(tray_name) = opts.tray.as_deref() {
                // Standard tray names map to IPP media-source
                // keywords; anything else keeps the loud refusal —
                // a tray must never be silently dropped (2026-11-05).
                match print_ipp::tray_to_media_source(tray_name) {
                    Some(source) => ipp_options.media_source = Some(source),
                    None => {
                        return Err(BackendError::fatal(
                            "IPP_OPTION_UNSUPPORTED",
                            &format!(
                                "tray {tray_name:?} cannot be mapped to an IPP media-source keyword"
                            ),
                        ));
                    }
                }
            }
            if opts.duplex {
                let sides = "two-sided-long-edge";
                if !caps.sides_supported.is_empty()
                    && !caps.sides_supported.iter().any(|x| x == sides)
                {
                    return Err(BackendError::fatal(
                        "IPP_OPTION_UNSUPPORTED",
                        "the IPP printer does not report duplex (two-sided) support",
                    ));
                }
                ipp_options.sides = Some(sides.to_string());
            }
            if let Some(paper_name) = opts.paper.as_deref() {
                let media = match paper_name.trim().to_ascii_lowercase().as_str() {
                    "a4" => Some("iso_a4_210x297mm"),
                    "a5" => Some("iso_a5_148x210mm"),
                    "letter" => Some("na_letter_8.5x11in"),
                    "legal" => Some("na_legal_8.5x14in"),
                    _ => None,
                };
                match media {
                    Some(m)
                        if caps.media_supported.is_empty()
                            || caps.media_supported.iter().any(|x| x == m) =>
                    {
                        ipp_options.media = Some(m.to_string());
                    }
                    _ => {
                        return Err(BackendError::fatal(
                            "IPP_OPTION_UNSUPPORTED",
                            &format!(
                                "paper {paper_name:?} cannot be mapped to a media size the IPP printer reports"
                            ),
                        ));
                    }
                }
            }
            if opts.color && !caps.color_supported {
                return Err(BackendError::fatal(
                    "IPP_OPTION_UNSUPPORTED",
                    "the IPP printer does not report colour support",
                ));
            }
            ipp_options.color_mode =
                Some(if opts.color { "color" } else { "monochrome" }.to_string());
            client
                .print_job_with_options(&data, fmt, title, opts.copies.max(1), &ipp_options)
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

        // When discovery returned an empty list the pattern is used
        // verbatim as the queue name and no kind was resolved: classify
        // from the name so the raw-to-virtual guard below still
        // applies instead of being skipped (fail-open edge).
        let target_kind = target_kind.or_else(|| Some(PrinterInfo::classify(&target_name, "")));

        // A document (PDF or image) in a raw-language mode
        // addressed to a virtual writer is re-routed to GDI
        // printing — the pages render faithfully at the printer's
        // paper size — instead of failing the job: virtual printers
        // are first-class silent targets in this product (the owner
        // prints receipts and reports to OneNote and PDF writers;
        // failing such a job in the field made the POS offer the
        // browser print dialog, 2026-11-04). Raw payloads — actual
        // printer-language bytes — keep the hard block below.
        if let Some(pk) = target_kind {
            if let Some(downgraded) =
                print_core::printer::document_mode_downgrade(&kind, &opts.mode, pk)
            {
                opts.mode = downgraded.to_string();
            }
        }

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

        // Live multi-condition health: ANY blocking condition
        // (paper out AND door open, toner empty AND paused, ...)
        // blocks the job, where the old single-status read could
        // only see the highest-priority flag. A failed read keeps
        // the discovery-time status checked above as the gate
        // rather than failing the job on a transient spooler
        // read error; a failed read is never reported as Ready.
        if let Ok(health) = print_win::printer_health(&target_name) {
            if health.blocks_printing() {
                let status = health.primary_status();
                return Err(BackendError::retryable(
                    Self::status_code(status),
                    &format!(
                        "printer {target_name:?} health blocks printing: {:?}",
                        health.conditions
                    ),
                ));
            }
        }

        let target_width_dots =
            if opts.mode == "escpos" && matches!(kind.as_str(), "pdf" | "image-png") {
                Some(Self::receipt_width_dots(
                    &target_name,
                    paper,
                    opts.media_roll_width_mm,
                    opts.source_width_mm,
                )?)
            } else {
                None
            };

        // Label modes: the fit box is the printer's largest discrete
        // form (or its roll width when only continuous media exists),
        // so a rendered label can never exceed the physical media.
        // The same capabilities read supplies the render DPI:
        // label bitmaps are emitted in device dots, so the box
        // and the render must use one resolution.
        let (label_box, render_dpi) = if matches!(opts.mode.as_str(), "zpl" | "tspl" | "epl")
            && matches!(kind.as_str(), "pdf" | "image-png")
        {
            match print_win::printer_capabilities(&target_name) {
                Ok(caps) => {
                    let dpi = print_core::printer::preferred_raw_render_dpi(&caps.resolutions_dpi);
                    let mut best: Option<(u32, u32)> = None;
                    let mut roll_w: Option<u32> = None;
                    for form in &caps.forms {
                        let w = (form.size.width_mm * dpi as f64 / 25.4) as u32;
                        if form.roll {
                            roll_w = Some(w);
                            continue;
                        }
                        let cand = (w, (form.size.height_mm * dpi as f64 / 25.4) as u32);
                        let area = cand.0 as u64 * cand.1 as u64;
                        if best.map_or(true, |b: (u32, u32)| area > b.0 as u64 * b.1 as u64) {
                            best = Some(cand);
                        }
                    }
                    (best.or_else(|| roll_w.map(|w| (w, u32::MAX))), dpi)
                }
                Err(_) => (None, 203),
            }
        } else {
            (None, 203)
        };

        match (kind.as_str(), opts.mode.as_str()) {
            ("pdf", "escpos" | "zpl" | "tspl" | "epl") => {
                let raw_bytes = Self::pdf_to_raw_stream(
                    &data,
                    RawRender {
                        mode: &opts.mode,
                        paper,
                        target_width_dots,
                        copies: opts.copies,
                        label_box,
                        render_dpi,
                        label_gap_mm: opts.label_gap_mm,
                    },
                    opts.cut,
                )?;
                print_win::print_raw(&target_name, title, &raw_bytes)
                    .map_err(|e| BackendError::ambiguous("SPOOLER_RAW_ERROR", &e.to_string()))?;
                Ok(())
            }
            ("pdf", "gdi") => {
                let policy = Self::parse_scale_policy(opts.scale_policy.as_deref());
                // Validate page geometry against the current Windows driver.
                // Both failures here are RETRYABLE (2026-11-05
                // practicality round): a capabilities read fails
                // transiently while the spooler/driver is busy or
                // initializing, and an empty form list at printer
                // power-on is the same condition one read later.
                // A genuinely broken driver still fails — after the
                // retry envelope, with the same codes.
                let capabilities =
                    print_win::printer_capabilities(&target_name).map_err(|error| {
                        BackendError::retryable(
                            "PRINTER_CAPABILITIES_UNAVAILABLE",
                            &format!(
                                "The Windows driver did not expose media capabilities: {error}"
                            ),
                        )
                    })?;
                if let Ok(info) = print_render::parse_pdf_info(&data) {
                    for page in &info.pages {
                        if capabilities.forms.is_empty() && !capabilities.custom_size_supported {
                            return Err(BackendError::retryable(
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
                } else if policy != ScalePolicy::CustomAllowed {
                    // The lightweight parser could not read this PDF:
                    // a strict paper policy must not silently skip the
                    // media validation this block exists to provide.
                    return Err(BackendError::fatal(
                        "PAPER_VALIDATION_UNAVAILABLE",
                        "the document's page sizes could not be verified against the printer's paper forms",
                    ));
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
                print_win::print_pdf_gdi_detailed(&target_name, title, &data, &gdi_opts).map_err(
                    |e| {
                        // ADR-0004 classification from the backend
                        // (audit M5): a failure after pages reached
                        // paper is ambiguous and must never auto-retry;
                        // a setup failure printed nothing, so the
                        // worker may retry it like any transient error.
                        if e.ambiguous {
                            BackendError::ambiguous("GDI_PRINT_ERROR", &e.message)
                        } else {
                            BackendError::retryable("GDI_SETUP_ERROR", &e.message)
                        }
                    },
                )?;
                Ok(())
            }
            ("pdf", _) => Err(BackendError::fatal(
                "UNKNOWN_MODE",
                &format!("unsupported print mode {:?} for this document", opts.mode),
            )),
            ("image-png", "gdi") => {
                let canvas = print_render::decode_png(&data)
                    .map_err(|e| BackendError::fatal("IMAGE_DECODE_ERROR", &e.to_string()))?;
                print_win::print_canvas_gdi_configured(
                    &target_name,
                    title,
                    &canvas,
                    opts.copies.max(1),
                    opts.tray.as_deref(),
                    opts.media_roll_width_mm,
                )
                .map_err(|e| {
                    if e.ambiguous {
                        BackendError::ambiguous("GDI_PRINT_ERROR", &e.message)
                    } else {
                        BackendError::retryable("GDI_SETUP_ERROR", &e.message)
                    }
                })?;
                Ok(())
            }
            ("image-png", "escpos" | "zpl" | "tspl" | "epl") => {
                let raw_bytes = Self::png_to_raw_stream(
                    &data,
                    RawRender {
                        mode: &opts.mode,
                        paper,
                        target_width_dots,
                        copies: opts.copies,
                        label_box,
                        render_dpi,
                        label_gap_mm: opts.label_gap_mm,
                    },
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
pub fn spawn_worker_loop(
    db_path: String,
    config_dir: std::path::PathBuf,
    wake: Arc<(Mutex<bool>, Condvar)>,
) {
    std::thread::spawn(move || {
        // Retention bounds: drop stored document bytes of jobs that
        // failed long ago, then — on a much longer window — the
        // history rows themselves, so the queue database cannot grow
        // for the life of the install (audit M1).
        if let Ok(mut retention_queue) = Queue::open(&db_path) {
            if let Err(e) = retention_queue.purge_stale_failed_payloads(30 * 86_400) {
                eprintln!("Yaseir Print: failed-payload retention purge failed: {e}");
            }
            if let Err(e) = retention_queue.purge_stale_failed_jobs(90 * 86_400) {
                eprintln!("Yaseir Print: failed-job row retention purge failed: {e}");
            }
        }
        let mut entropy: u64 = 0x5EED_2026;
        loop {
            let mut did_work = false;
            match Queue::open(&db_path) {
                Ok(mut worker_queue) => {
                    // Fail jobs wedged in a working state by a dead or
                    // stuck worker (RECONCILED_STALE) instead of letting
                    // them hang forever; fresh jobs are never touched.
                    if let Err(e) = worker_queue.reconcile_stale() {
                        eprintln!("Yaseir Print: stale-job reconciliation failed: {e}");
                    }
                    let printers = match worker_queue.queued_printers() {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("Yaseir Print: queued_printers failed: {e}");
                            Vec::new()
                        }
                    };
                    for printer in printers {
                        entropy = entropy.wrapping_mul(6364136223846793005).wrapping_add(1);
                        let mut worker = Worker {
                            queue: &mut worker_queue,
                            backend: NativePrintBackend {
                                db_path: db_path.clone(),
                                config_dir: config_dir.clone(),
                            },
                            cfg: WorkerConfig::default(),
                            entropy,
                        };
                        // Panic isolation: one bad job or backend panic
                        // must never kill the only worker thread and
                        // silently stop all printing for the session.
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                worker.run_once(&printer)
                            }));
                        match outcome {
                            Ok(Ok(o)) => {
                                if o != Outcome::Idle {
                                    did_work = true;
                                }
                            }
                            Ok(Err(e)) => {
                                eprintln!("Yaseir Print: worker error on {printer}: {e}");
                            }
                            Err(_) => {
                                eprintln!("Yaseir Print: worker panicked on {printer}; continuing");
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Yaseir Print: cannot open the print queue: {e}");
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
    fn receipt_width_evidence_choice() {
        // Form evidence wins when it roughly agrees with the
        // receipt (80 mm form, 79.4 mm receipt -> 576 dots).
        assert_eq!(
            NativePrintBackend::form_width_dots(Some(80.0), Some(79.4)),
            Some(576)
        );
        // A form list describing other media entirely (A4 forms
        // for an 80 mm receipt) is discarded, not obeyed.
        assert_eq!(
            NativePrintBackend::form_width_dots(Some(210.0), Some(79.4)),
            None
        );
        // No form evidence: no dots from this step (the caller
        // falls through to the receipt's own width).
        assert_eq!(NativePrintBackend::form_width_dots(None, Some(80.0)), None);
        // Non-catalogue form widths convert through the general
        // printable-width model.
        assert_eq!(
            NativePrintBackend::form_width_dots(Some(100.0), None),
            Some(728)
        );
    }

    #[test]
    fn tab_titles_and_pos_detection() {
        // Tab titles are derived from routes, never from page text.
        assert_eq!(
            tab_title_for("http://192.168.1.20:8069/pos/ui"),
            "Point of Sale"
        );
        assert_eq!(
            tab_title_for("https://odoo.example.com/web/login"),
            "Sign in"
        );
        assert_eq!(
            tab_title_for("https://odoo.example.com/odoo/sales"),
            "Sales"
        );
        assert_eq!(tab_title_for("https://odoo.example.com/odoo"), "Odoo");
        assert_eq!(
            tab_title_for("tauri://localhost/index.html"),
            "Control Center"
        );
        assert_eq!(tab_title_for("tauri://localhost/boot.html"), "Yaseir Print");
        // POS detection: /pos and /pos/* only — a backend route that
        // merely contains "pos" must not divert into a POS tab.
        assert!(is_pos_url("http://h/pos/ui"));
        assert!(is_pos_url("http://h/pos"));
        assert!(!is_pos_url("http://h/odoo/point-of-sale"));
        assert!(!is_pos_url("http://h/postal"));
        // Registry: snapshot marks the POS tab and the active tab.
        let mut tabs = TabsState {
            next_id: 2,
            ..Default::default()
        };
        tabs.register("tab-1", "http://h/odoo");
        tabs.register("tab-2", "http://h/pos/ui");
        tabs.active = Some("tab-2".to_string());
        let snap = tabs.snapshot();
        assert_eq!(snap.len(), 2);
        assert!(!snap[0].is_pos && !snap[0].active);
        assert!(snap[1].is_pos && snap[1].active);
        assert!(tabs.is_pos_tab("tab-2"));
        assert!(!tabs.is_pos_tab("tab-1"));
    }

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

    #[test]
    fn ipp_uri_brackets_ipv6_hosts() {
        // parse_host_port strips the brackets; the printer-uri
        // sent to the printer must put them back, or the URI is
        // malformed and the printer rejects the job.
        let (h, p, path) =
            NativePrintBackend::parse_host_port("ipp://[2001:db8::1]:631/ipp/print", 631).unwrap();
        assert_eq!(
            NativePrintBackend::ipp_uri(&h, p, &path),
            "ipp://[2001:db8::1]:631/ipp/print"
        );
        assert_eq!(
            NativePrintBackend::ipp_uri("192.168.1.50", 631, "/ipp/print"),
            "ipp://192.168.1.50:631/ipp/print"
        );
        assert_eq!(
            NativePrintBackend::ipp_uri("printer.local", 631, "/ipp/print"),
            "ipp://printer.local:631/ipp/print"
        );
    }

    #[test]
    fn label_gap_helpers_default_to_3mm() {
        assert_eq!(NativePrintBackend::label_gap_mm(None), 3.0);
        assert_eq!(NativePrintBackend::label_gap_mm(Some(2.0)), 2.0);
        assert_eq!(NativePrintBackend::label_gap_mm(Some(-1.0)), 0.0);
        // 3 mm at 203 dpi is the EPL default of 24 dots.
        assert_eq!(NativePrintBackend::label_gap_dots(None, 203), 24);
        assert_eq!(NativePrintBackend::label_gap_dots(Some(3.0), 300), 35);
    }
}

//! Tauri commands callable from the configured Odoo origin.
//! Every command validates the calling webview's origin in Rust first
//! (ADR-0002): Tauri app commands are not permission-gated by default,
//! and origin classification has been wrong before (CVE-2026-42184).

use print_core::security::{origin_is_allowed, validate_job_input};
use print_core::{Job, JobState, Queue};
use serde::Deserialize;
use std::sync::Mutex;
use tauri::State;

pub struct AppState {
    pub queue: Mutex<Queue>,
    pub configured_origin: String,
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
    pub url: String,
    // NOTE (spike S4): bytes currently arrive through Tauri's standard
    // invoke serialisation. The raw-body transfer (<2s for 50 MB,
    // bounded memory) is the Windows-phase optimisation tracked in
    // docs/spikes/S1-S8.md; the 50 MiB cap below applies either way.
    pub bytes: Vec<u8>,
}

fn check_origin(webview: &tauri::Webview, configured: &str) -> Result<(), String> {
    let url = webview.url().map_err(|e| e.to_string())?;
    let origin = url.origin().ascii_serialization();
    if origin_is_allowed(configured, &origin) {
        Ok(())
    } else {
        Err("forbidden_origin".to_string())
    }
}

#[tauri::command]
pub fn print_pdf(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    req: PrintPdfRequest,
) -> Result<String, String> {
    check_origin(&webview, &state.configured_origin)?;
    // Inspect the document before queueing it: a non-PDF, an encrypted
    // PDF, or a limit violation is rejected here, never at the printer.
    let info = print_render::parse_pdf_info(&req.bytes).map_err(|e| e.to_string())?;
    let pages = info.pages.len() as u32;
    let first = info.pages.first().ok_or("empty_document")?;
    validate_job_input(req.bytes.len(), pages, req.copies, first.size)
        .map_err(|e| format!("{e:?}"))?;
    let job = Job {
        id: req.idempotency_key.clone(),
        idempotency_key: req.idempotency_key.clone(),
        printer: req.printer_pattern.clone(),
        report: Some(req.report_name.clone()),
        pages,
        bytes: req.bytes.len() as u64,
        // Receipt-style modes jump the queue; office documents are normal.
        priority: if req.mode == "escpos" { 10 } else { 0 },
        state: JobState::Queued,
        attempts: 0,
        error_code: None,
    };
    let mut queue = state
        .queue
        .lock()
        .map_err(|_| "queue_poisoned".to_string())?;
    let id = queue.enqueue(&job).map_err(|e| e.to_string())?;
    // The document bytes are stored with the job so the worker can print
    // after any restart. On a deduplicated submission the payload of the
    // original job is kept (identical key = identical document contract).
    queue
        .store_payload(&id, "pdf", &req.bytes)
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintTextRequest {
    pub idempotency_key: String,
    pub report_name: String,
    pub printer_pattern: String,
    /// Plain text of a qweb-text report (kitchen tickets). Arabic text is
    /// NOT sent through this path (ESC/POS text mode cannot shape RTL);
    /// Arabic receipts use the PDF/raster path by design.
    pub text: String,
    pub cut: bool,
}

/// qweb-text path: encode the text as an ESC/POS stream here (text lines,
/// feed, cut) and queue it as a raw job with receipt priority.
#[tauri::command]
pub fn print_text(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    req: PrintTextRequest,
) -> Result<String, String> {
    check_origin(&webview, &state.configured_origin)?;
    if req.text.is_empty() || req.text.len() > 1_048_576 {
        return Err("bad_text_size".to_string());
    }
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
    let mut queue = state
        .queue
        .lock()
        .map_err(|_| "queue_poisoned".to_string())?;
    let id = queue.enqueue(&job).map_err(|e| e.to_string())?;
    queue
        .store_payload(&id, "escpos", &bytes)
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintImageRequest {
    pub idempotency_key: String,
    /// Source name, e.g. "point_of_sale.receipt".
    pub report_name: String,
    pub printer_pattern: String,
    /// Receipt paper from the server profile: "80mm" (default) or "58mm".
    pub paper: Option<String>,
    /// PNG bytes captured by the POS extension (receipt as an image).
    pub bytes: Vec<u8>,
}

/// POS receipt / image path: the addon sends the receipt as a PNG image;
/// we validate it by fully decoding it here (bad images are rejected at
/// the door, never at the printer), then queue it durably. At print
/// time the backend converts it with print_render::receipt_png_to_escpos
/// (scale to the paper width, threshold, ESC/POS raster) for thermal
/// profiles, or prints the bitmap via GDI for office profiles.
#[tauri::command]
pub fn print_image(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    req: PrintImageRequest,
) -> Result<String, String> {
    check_origin(&webview, &state.configured_origin)?;
    let _img = print_render::decode_png(&req.bytes).map_err(|e| e.to_string())?;
    let job = Job {
        id: req.idempotency_key.clone(),
        idempotency_key: req.idempotency_key.clone(),
        printer: req.printer_pattern.clone(),
        report: Some(req.report_name.clone()),
        pages: 1,
        bytes: req.bytes.len() as u64,
        // Receipts jump the queue, like ESC/POS report jobs.
        priority: 10,
        state: JobState::Queued,
        attempts: 0,
        error_code: None,
    };
    let mut queue = state
        .queue
        .lock()
        .map_err(|_| "queue_poisoned".to_string())?;
    let id = queue.enqueue(&job).map_err(|e| e.to_string())?;
    queue
        .store_payload(&id, "image-png", &req.bytes)
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn print_status(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<String, String> {
    check_origin(&webview, &state.configured_origin)?;
    let queue = state
        .queue
        .lock()
        .map_err(|_| "queue_poisoned".to_string())?;
    let job = queue.get(&job_id).map_err(|e| e.to_string())?;
    Ok(job.state.as_str().to_string())
}

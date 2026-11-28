//! Tauri commands callable from the configured Odoo origin and the local
//! Control Center (`index.html`).
//! Every command validates the calling webview's origin in Rust first
//! (ADR-0002): Tauri app commands are registered in `AppManifest::commands`
//! (`build.rs`) and also checked explicitly in Rust so origin confusion
//! (CVE-2026-42184) can never bypass security.

use crate::config;
use print_core::diagnostics::{build_bundle_from_queue, JobSummary};
use print_core::paper::{Form, ScalePolicy};
use print_core::printer::{PrinterInfo, PrinterKind};
use print_core::routing::resolve_printer;
use print_core::security::{origin_is_allowed, validate_job_input};
use print_core::worker::Outcome;
use print_core::{
    BackendError, Job, JobOptions, JobState, PrintBackend, Queue, Worker, WorkerConfig,
};
use print_raw::MonoBitmap;
use print_render::RenderMode;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
            eprintln!("Itbaa: queue mutex was poisoned; recovering the guard");
            poisoned.into_inner()
        }
    }
}

/// Enqueue job + payload + options in one SQLite transaction; duplicate
/// idempotency keys return the original job id untouched.
fn enqueue_idempotent(
    queue: &Mutex<Queue>,
    job: &Job,
    kind: &str,
    bytes: &[u8],
    opts: &JobOptions,
) -> Result<String, String> {
    let mut q = lock_queue(queue);
    match q
        .enqueue_with_payload(job, kind, bytes, opts)
        .map_err(|e| e.to_string())?
    {
        Some(new_id) => Ok(new_id),
        None => q
            .job_id_for_key(&job.idempotency_key)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "queue_inconsistent".to_string()),
    }
}

pub fn read_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("Itbaa: RwLock (read) was poisoned; recovering the guard");
            poisoned.into_inner()
        }
    }
}

pub fn write_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("Itbaa: RwLock (write) was poisoned; recovering the guard");
            poisoned.into_inner()
        }
    }
}

/// TCP 9100 transport budgets: a connect that takes longer is
/// a printer that is off or unreachable (its failure proves zero
/// bytes were sent, so the job may retry); the write budget
/// covers a full document stream to a slow printer (a failure
/// inside it is ambiguous, ADR-0004).
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TCP_WRITE_TIMEOUT: Duration = Duration::from_secs(20);
/// DLE EOT live-status probe budget per frame query: long
/// enough for a healthy printer to answer, short enough to
/// never noticeably delay the job it guards.
const DLE_PROBE_TIMEOUT: Duration = Duration::from_millis(350);
/// TTL for skipping DLE EOT probes on silent or briefly unreachable TCP 9100
/// printers: long enough to avoid per-receipt stalls, short enough that a
/// printer coming back online recovers live hardware health checks.
const DLE_SILENT_TTL: Duration = Duration::from_secs(300);
const DLE_SILENT_MAX_ENTRIES: usize = 256;

fn dle_silent_is_active(map: &mut HashMap<String, Instant>, key: &str, now: Instant) -> bool {
    if let Some(&marked_at) = map.get(key) {
        if now.saturating_duration_since(marked_at) < DLE_SILENT_TTL {
            return true;
        }
        map.remove(key);
    }
    false
}

fn dle_silent_record(map: &mut HashMap<String, Instant>, key: String, now: Instant) {
    if !map.contains_key(&key) && map.len() >= DLE_SILENT_MAX_ENTRIES {
        map.retain(|_, ts| now.saturating_duration_since(*ts) < DLE_SILENT_TTL);
        if map.len() >= DLE_SILENT_MAX_ENTRIES {
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, ts)| **ts)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
    }
    map.insert(key, now);
}
/// Cap expanded multi-copy raw byte streams to prevent OOM on large payloads * copies.
const MAX_RAW_EXPANDED_BYTES: usize = 100 * 1024 * 1024;
const MAX_CONCURRENT_PRINTERS: usize = 8;
const WORKER_ERROR_BACKOFF: Duration = Duration::from_secs(5);

pub struct AppState {
    pub queue: Arc<Mutex<Queue>>,
    pub configured_origin: Arc<RwLock<Option<String>>>,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub locked_by_policy: bool,
    pub auto_reprint: RwLock<bool>,
    pub license_key: Arc<RwLock<Option<String>>>,
    pub license_locked_by_policy: bool,
    pub last_seen_utc: Arc<RwLock<u64>>,
    pub allow_exit: Arc<AtomicBool>,
    pub worker_wake: Arc<(Mutex<bool>, Condvar)>,
    pub worker_stop: Arc<AtomicBool>,
    pub printer_settings: Arc<RwLock<config::PrinterSettingsMap>>,
    /// Latest parsed environment probe report from the page:
    /// the JavaScript feature/storage facts the Odoo frontend depends
    /// on, surfaced in the overview and the diagnostics bundle so a
    /// broken webview environment is a fact, not a guess.
    pub environment: RwLock<Option<serde_json::Value>>,
    /// Tab registry for the tabbed browser (owner brief,
    /// 2026-11-05): one window, a local tab bar, and one child
    /// webview per tab, all sharing one data directory so Odoo
    /// sessions and cookies behave as in a single browser profile.
    pub tabs: RwLock<TabsState>,
    pub started_at: Instant,
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
    pub pos_tabs: std::collections::HashSet<String>,
}

impl TabsState {
    pub fn register(&mut self, label: &str, url: &str) {
        if !self.order.iter().any(|l| l == label) {
            self.order.push(label.to_string());
        }
        self.urls.insert(label.to_string(), url.to_string());
        if is_pos_url(url) {
            self.pos_tabs.insert(label.to_string());
        }
        if self.active.is_none() {
            self.active = Some(label.to_string());
        }
    }

    pub fn set_url(&mut self, label: &str, url: &str) {
        if !self.order.iter().any(|l| l == label) {
            return;
        }
        self.urls.insert(label.to_string(), url.to_string());
        if is_pos_url(url) {
            self.pos_tabs.insert(label.to_string());
        } else if !url.contains("/web/login") {
            self.pos_tabs.remove(label);
        }
    }

    /// Move a tab to a new position in the strip (drag-and-drop
    /// reorder from the tab bar). The index is clamped to the
    /// strip; moving a tab never changes which tab is active.
    pub fn move_to(&mut self, label: &str, to_index: usize) -> bool {
        let Some(from) = self.order.iter().position(|l| l == label) else {
            return false;
        };
        let item = self.order.remove(from);
        let at = to_index.min(self.order.len());
        self.order.insert(at, item);
        true
    }

    pub fn url_of(&self, label: &str) -> Option<&str> {
        self.urls.get(label).map(|s| s.as_str())
    }

    pub fn is_pos_tab(&self, label: &str) -> bool {
        self.pos_tabs.contains(label) || self.url_of(label).map(is_pos_url).unwrap_or(false)
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
                    is_pos: self.is_pos_tab(label),
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
        let p = u.path();
        if p == "/" || p.ends_with("/index.html") || url.contains("index.html") {
            return "Control Center".to_string();
        }
        if p.ends_with("/settings.html") || url.contains("settings.html") {
            return "Settings".to_string();
        }
        return "Itbaa".to_string();
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

fn reconcile_default_flag(printers: &mut [PrinterInfo], default_name: Option<&str>) {
    let Some(def) = default_name.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    for p in printers.iter_mut() {
        if p.name == def {
            p.is_default = true;
        }
    }
}

/// Choose the label fit box (dots) and render DPI for ZPL/TSPL/EPL/CPCL PDF/image jobs.
/// Ignores generic Windows office sheet forms (`Letter`, `Ledger`, `A4`, ...) and
/// caps raw label rendering at 300 DPI so `Generic / Text Only` drivers never emit
/// multi-megabyte 600 DPI full-sheet bitmaps.
fn resolve_label_box_and_dpi(caps: &print_core::PrinterCapabilities) -> (Option<(u32, u32)>, u32) {
    let label_forms: Vec<_> = caps
        .forms
        .iter()
        .filter(|f| !NativePrintBackend::is_office_sheet_or_envelope(&f.name))
        .collect();
    let raw_dpi = print_core::printer::preferred_raw_render_dpi(&caps.resolutions_dpi);
    let dpi = if label_forms.is_empty() && raw_dpi > 300 {
        203
    } else {
        raw_dpi.min(300)
    };
    let mut best: Option<(u32, u32)> = None;
    let mut roll_w: Option<u32> = None;
    for form in label_forms {
        let w = (form.size.width_mm * dpi as f64 / 25.4) as u32;
        if w == 0 {
            continue;
        }
        if form.roll {
            roll_w = Some(w);
            continue;
        }
        let h = (form.size.height_mm * dpi as f64 / 25.4) as u32;
        if h == 0 {
            continue;
        }
        let cand = (w, h);
        let area = cand.0 as u64 * cand.1 as u64;
        if best.map_or(true, |b: (u32, u32)| area > b.0 as u64 * b.1 as u64) {
            best = Some(cand);
        }
    }
    let fallback_box = || {
        let w = (102.0 * dpi as f64 / 25.4).round() as u32;
        let h = (152.0 * dpi as f64 / 25.4).round() as u32;
        Some((w, h))
    };
    let box_dots = best
        .or_else(|| roll_w.map(|w| (w, u32::MAX)))
        .or_else(fallback_box);
    (box_dots, dpi)
}

/// Single-source printer enumeration backed by `print_win::DISCOVERY_CACHE` (12 s TTL).
/// Eliminates the former 20 s + 30 s stacked cache that caused live printers to exceed
/// `PrinterHealth::ttl_secs` (30 s) and flash "Outdated / بيانات قديمة" in the UI.
/// Passing `force = true` clears both discovery and capability caches before reading.
async fn cached_printers(force: bool) -> Result<(Vec<PrinterInfo>, Option<String>), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if force {
            print_win::invalidate_discovery_cache();
        }
        let mut printers = print_win::discover_printers().map_err(|e| e.to_string())?;
        let default = print_win::default_printer_name().ok();
        reconcile_default_flag(&mut printers, default.as_deref());
        Ok::<_, String>((printers, default))
    })
    .await
    .map_err(|e| e.to_string())?
}

static PDFIUM_READY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// PDFium availability is a process-lifetime fact; probing it used to
/// load the 7.5 MB DLL on every overview call.
pub(crate) fn pdfium_ready() -> bool {
    *PDFIUM_READY.get_or_init(|| print_render::Pdfium::load().is_ok())
}

/// WebView2 runtime version is a process-lifetime constant; caching
/// avoids querying the runtime on every Control Center poll.
fn webview_version_cached() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| tauri::webview_version().unwrap_or_else(|_| "unknown".to_string()))
        .as_str()
}

impl AppState {
    pub fn notify_worker(&self) {
        let (lock, cvar) = &*self.worker_wake;
        let mut pending = lock.lock().unwrap_or_else(|p| p.into_inner());
        *pending = true;
        cvar.notify_all();
    }
}

/// Legacy fallback cap for JS `Uint8Array` serialized as a JSON object.
/// `BRIDGE_JS` always base64-encodes binary payloads; capping the map path prevents BTreeMap OOM.
const MAX_BYTE_MAP_LEN: usize = 1_048_576;

/// Custom deserializer that accepts:
/// 1. Standard JSON number array `[37, 80, 68, 70, ...]`
/// 2. Base64 string `"JVBERi0..."`
/// 3. JavaScript `Uint8Array` serialized as an object `{"0": 37, "1": 80, ...}`
fn deserialize_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    struct BytesVisitor;

    impl<'de> serde::de::Visitor<'de> for BytesVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a byte array, base64 string, or contiguous byte index map")
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
            decode_base64(v).map_err(E::custom)
        }

        fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
            if v.len() > print_core::security::MAX_JOB_BYTES {
                return Err(E::custom("payload_too_large"));
            }
            Ok(v.to_vec())
        }

        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let cap = seq
                .size_hint()
                .unwrap_or(0)
                .min(print_core::security::MAX_JOB_BYTES);
            let mut out = Vec::with_capacity(cap);
            while let Some(b) = seq.next_element::<u8>()? {
                if out.len() >= print_core::security::MAX_JOB_BYTES {
                    return Err(serde::de::Error::custom("payload_too_large"));
                }
                out.push(b);
            }
            Ok(out)
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut m = BTreeMap::<usize, u8>::new();
            while let Some((k, v)) = map.next_entry::<String, u8>()? {
                let idx = k
                    .parse::<usize>()
                    .map_err(|_| serde::de::Error::custom("invalid byte map key"))?;
                if idx >= MAX_BYTE_MAP_LEN || m.len() >= MAX_BYTE_MAP_LEN {
                    return Err(serde::de::Error::custom("payload_too_large"));
                }
                m.insert(idx, v);
            }
            if m.keys().enumerate().any(|(i, k)| *k != i) {
                return Err(serde::de::Error::custom(
                    "sparse byte map payload: indices must be contiguous from 0",
                ));
            }
            Ok(m.into_values().collect())
        }
    }

    deserializer.deserialize_any(BytesVisitor)
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let raw = if input.starts_with("data:") {
        input
            .split_once(";base64,")
            .map(|(_, b)| b)
            .unwrap_or(input)
    } else {
        input
    };
    let max_b64_len = (print_core::security::MAX_JOB_BYTES / 3 + 1) * 4;
    let mut clean = Vec::with_capacity(raw.len().min(max_b64_len));
    for b in raw.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if clean.len() >= max_b64_len {
            return Err("payload_too_large".to_string());
        }
        clean.push(b);
    }
    if clean.is_empty() {
        return Ok(Vec::new());
    }
    if clean.len() % 4 == 1 {
        return Err("invalid base64 length".to_string());
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
    let num_chunks = clean.len().div_ceil(4);
    for (idx, chunk) in clean.chunks(4).enumerate() {
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        out.push((a << 2) | (b >> 4));
        if chunk.len() > 2 && chunk[2] != b'=' {
            let c = val(chunk[2])?;
            out.push((b << 4) | (c >> 2));
            if chunk.len() > 3 && chunk[3] != b'=' {
                let d = val(chunk[3])?;
                out.push((c << 6) | d);
            }
        } else if idx + 1 < num_chunks || (chunk.len() > 3 && chunk[3] != b'=') {
            return Err("unexpected base64 padding".to_string());
        }
    }
    Ok(out)
}

/// Exact check for Tauri's internal bundled asset origin (`tauri://localhost`
/// or `http(s)://tauri.localhost`). Uses full host equality so CVE-2026-42184
/// first-label tricks (`http://tauri.evil.com`) never match.
pub fn is_local_app_url(url: &tauri::Url) -> bool {
    if url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "http" | "https" => url.host_str() == Some("tauri.localhost"),
        _ => false,
    }
}

pub(crate) fn local_app_url(file: &str) -> Result<tauri::Url, String> {
    let raw = if cfg!(windows) {
        format!("http://tauri.localhost/{file}")
    } else {
        format!("tauri://localhost/{file}")
    };
    raw.parse().map_err(|_| "bad_local_url".to_string())
}

fn check_origin(webview: &tauri::Webview, state: &AppState) -> Result<(), String> {
    // shortcut: Tauri 2.12 attributes IPC via webview.url() (top-frame URL), not per-iframe origin (ADR-0005); upgrade when Tauri exposes frame-level IPC origin.
    let url = webview.url().map_err(|e| e.to_string())?;
    if is_local_app_url(&url) {
        return Ok(());
    }
    let origin = url.origin().ascii_serialization();
    let guard = read_lock(&state.configured_origin);
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

fn is_loopback_host(host: &str) -> bool {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => v4.is_loopback(),
            std::net::IpAddr::V6(v6) => {
                if v6.is_loopback() {
                    return true;
                }
                let o = v6.octets();
                let embedded = (o[..10] == [0u8; 10] && o[10] == 0xff && o[11] == 0xff)
                    || o[..12] == [0u8; 12];
                embedded && o[12] == 127
            }
        };
    }
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    h == "localhost" || h.ends_with(".localhost")
}

fn parse_printer_uri_target(trimmed: &str) -> Result<Option<(String, u16, bool, u16)>, String> {
    let lower = trimmed.to_ascii_lowercase();
    let (scheme_len, default_port, is_raw_tcp) =
        if lower.starts_with("tcp://") || lower.starts_with("raw://") {
            (6usize, 9100u16, true)
        } else if lower.starts_with("ipp://") {
            (6usize, 631u16, false)
        } else if lower.starts_with("http://") {
            (7usize, 631u16, false)
        } else if lower.starts_with("https://") {
            (8usize, 631u16, false)
        } else {
            return Ok(None);
        };
    let normalized = format!("{}{}", &lower[..scheme_len], &trimmed[scheme_len..]);
    let (host, port, _) = NativePrintBackend::parse_host_port(&normalized, default_port)
        .map_err(|e| format!("{}: {}", e.code, e.message))?;
    Ok(Some((host, port, is_raw_tcp, default_port)))
}

fn printer_port_allowed(port: u16, is_raw_tcp: bool) -> bool {
    if is_raw_tcp {
        (9100..=9109).contains(&port)
    } else {
        port == 631
    }
}

fn is_discovered_loopback_printer(
    printer_pattern: &str,
    discovered: &[print_core::PrinterInfo],
) -> bool {
    let Ok(Some((host, port, is_raw_tcp, default_port))) =
        parse_printer_uri_target(printer_pattern.trim())
    else {
        return false;
    };
    if !is_loopback_host(&host) || !printer_port_allowed(port, is_raw_tcp) {
        return false;
    }
    discovered.iter().any(|p| {
        let port_raw = p.port.trim();
        let port_spec = port_raw.strip_prefix("IP_").unwrap_or(port_raw);
        NativePrintBackend::parse_host_port(&port_spec.to_ascii_lowercase(), default_port)
            .map(|(p_host, p_port, _)| p_host.eq_ignore_ascii_case(&host) && p_port == port)
            .unwrap_or(false)
    })
}

fn validate_printer_pattern(
    printer_pattern: &str,
    is_local: bool,
    configured: &config::PrinterSettingsMap,
) -> Result<(), String> {
    let trimmed = printer_pattern.trim();
    if trimmed.is_empty() || printer_pattern.len() > 512 {
        return Err("printer_required".to_string());
    }
    if is_local || configured.contains_key(trimmed) || configured.contains_key(printer_pattern) {
        return Ok(());
    }
    let Some((host, port, is_raw_tcp, _)) = parse_printer_uri_target(trimmed)? else {
        return Ok(());
    };
    if is_loopback_host(&host) {
        return Err(
            "BAD_PRINTER_ADDRESS: loopback printer address is not allowed from remote pages"
                .to_string(),
        );
    }
    if !printer_port_allowed(port, is_raw_tcp) {
        return Err(
            "BAD_PRINTER_ADDRESS: printer port is not allowed from remote pages".to_string(),
        );
    }
    Ok(())
}

fn check_printer_pattern(
    webview: &tauri::Webview,
    state: &AppState,
    printer_pattern: &str,
) -> Result<(), String> {
    let is_local = webview.url().map(|u| is_local_app_url(&u)).unwrap_or(false);
    let configured = read_lock(&state.printer_settings);
    match validate_printer_pattern(printer_pattern, is_local, &configured) {
        Ok(()) => Ok(()),
        Err(err) => {
            // Allow loopback printer URIs that correspond to an actual OS-installed printer port
            // (e.g. USB-to-TCP / local virtual queue bound on 127.0.0.1).
            let discovered = print_win::discover_printers().unwrap_or_default();
            if is_discovered_loopback_printer(printer_pattern, &discovered) {
                Ok(())
            } else {
                Err(err)
            }
        }
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
    /// Tri-state (2026-11-07): absent/null = nobody chose, and
    /// the driver default applies at the printer; true/false are
    /// explicit choices. Older pages send a boolean, which
    /// deserializes as Some(..) — their behaviour is unchanged.
    #[serde(default)]
    pub duplex: Option<bool>,
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
    #[serde(deserialize_with = "deserialize_bytes")]
    pub bytes: Vec<u8>,
}

fn default_true() -> bool {
    true
}

/// Offline Ed25519 license gate (<1ms) enforced before accepting print jobs.
/// Updates monotonic `last_seen_utc` on valid checks to detect clock rollback.
pub fn check_license(state: &AppState) -> Result<print_core::license::LicensePayload, String> {
    let origin = read_lock(&state.configured_origin)
        .clone()
        .ok_or_else(|| print_core::license::ERR_LICENSE_REQUIRED.to_string())?;
    let key = read_lock(&state.license_key).clone();
    let now = print_core::license::now_utc_secs();
    let last_seen = *read_lock(&state.last_seen_utc);
    let res = print_core::license::verify_license(key.as_deref(), &origin, now, last_seen);
    match res {
        Ok(payload) => {
            if now > last_seen {
                *write_lock(&state.last_seen_utc) = now;
                let dir = state.config_dir.clone();
                std::thread::spawn(move || config::persist_last_seen_utc(&dir, now));
            }
            Ok(payload)
        }
        Err(err) => Err(err.code().to_string()),
    }
}

#[tauri::command]
pub fn license_status_cmd(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<print_core::license::LicenseStatus, String> {
    check_local_only(&webview)?;
    let origin = read_lock(&state.configured_origin)
        .clone()
        .unwrap_or_default();
    let key = read_lock(&state.license_key).clone();
    let now = print_core::license::now_utc_secs();
    let last_seen = *read_lock(&state.last_seen_utc);
    let res = print_core::license::verify_license(key.as_deref(), &origin, now, last_seen);
    Ok(print_core::license::LicenseStatus::from_result(&res))
}

#[tauri::command]
pub async fn save_license_cmd(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    key: String,
) -> Result<print_core::license::LicenseStatus, String> {
    check_local_only(&webview)?;
    if state.license_locked_by_policy {
        return Err("locked_by_policy".to_string());
    }
    let origin = read_lock(&state.configured_origin)
        .clone()
        .ok_or_else(|| "no_origin_configured".to_string())?;
    let now = print_core::license::now_utc_secs();
    let last_seen = *read_lock(&state.last_seen_utc);
    let trimmed = key.trim().to_string();
    let res = print_core::license::verify_license(Some(&trimmed), &origin, now, last_seen);
    let status = print_core::license::LicenseStatus::from_result(&res);
    if !status.valid {
        return Err(status.code);
    }
    let config_dir = state.config_dir.clone();
    let key_clone = trimmed.clone();
    tauri::async_runtime::spawn_blocking(move || config::save_license_key(&config_dir, &key_clone))
        .await
        .map_err(|e| e.to_string())??;
    *write_lock(&state.license_key) = Some(trimmed);
    if now > last_seen {
        *write_lock(&state.last_seen_utc) = now;
        let dir = state.config_dir.clone();
        let _ =
            tauri::async_runtime::spawn_blocking(move || config::persist_last_seen_utc(&dir, now))
                .await;
    }
    state.notify_worker();
    Ok(status)
}

#[tauri::command]
pub async fn print_pdf(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    req: PrintPdfRequest,
) -> Result<String, String> {
    check_origin(&webview, &state)?;
    check_license(&state)?;
    // Validate everything that does not need the document BEFORE any
    // parser touches untrusted bytes (size, copies, key shape).
    if !print_core::security::valid_idempotency_key(&req.idempotency_key) {
        return Err("bad_idempotency_key".to_string());
    }
    check_printer_pattern(&webview, &state, &req.printer_pattern)?;
    if req.report_name.len() > 512 {
        return Err("bad_report_name".to_string());
    }
    let copies = req.copies.max(1);
    print_core::security::validate_payload_limits(req.bytes.len(), copies)
        .map_err(|e| format!("{e:?}"))?;
    let queue_arc = Arc::clone(&state.queue);
    let id = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let pages = match print_render::parse_pdf_info(&req.bytes) {
            Ok(info) => {
                let pages = info.pages.len() as u32;
                if pages == 0 {
                    return Err("empty_document".to_string());
                }
                for p in &info.pages {
                    validate_job_input(req.bytes.len(), pages, copies, p.size)
                        .map_err(|e| format!("{e:?}"))?;
                }
                pages
            }
            Err(parse_err) => {
                // Lightweight parser can fail on compressed xref/object streams;
                // fall back to PDFium (when loaded) or %PDF- header verification.
                let has_magic = req.bytes.windows(5).take(1024).any(|w| w == b"%PDF-");
                if !has_magic {
                    return Err(parse_err.to_string());
                }
                // Defer PDFium document load to the worker thread so enqueue returns
                // immediately without contending on PDFium's global lock; the worker
                // enforces page count > 0 and <= MAX_PAGES before rendering.
                1
            }
        };
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
            copies,
            scale_policy: req.scale_policy.clone(),
            duplex: req.duplex,
            color: req.color,
            cut: true,
            tray: req.tray.clone(),
            media_roll_width_mm: req
                .media_roll_width_mm
                .filter(|w| w.is_finite() && (20.0..=500.0).contains(w)),
            label_gap_mm: req
                .label_gap_mm
                .filter(|g| g.is_finite() && (0.0..=100.0).contains(g)),
            source_width_mm: None,
        };
        enqueue_idempotent(&queue_arc, &job, "pdf", &req.bytes, &opts)
    })
    .await
    .map_err(|e| e.to_string())??;
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
    pub mode: Option<String>,
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
    check_license(&state)?;
    // Same key validation as print_pdf/print_image (audit M1): an
    // unvalidated key becomes the job id verbatim, so an empty or
    // colliding key would silently dedupe two different documents
    // into one job.
    if !print_core::security::valid_idempotency_key(&req.idempotency_key) {
        return Err("bad_idempotency_key".to_string());
    }
    check_printer_pattern(&webview, &state, &req.printer_pattern)?;
    if req.report_name.len() > 512 {
        return Err("bad_report_name".to_string());
    }
    if req.text.is_empty() || req.text.len() > 1_048_576 {
        return Err("bad_text_size".to_string());
    }
    let copies = req.copies.unwrap_or(1).max(1);
    validate_job_input(
        req.text.len(),
        1,
        copies,
        print_core::PageSize::roll_80(200.0),
    )
    .map_err(|e| format!("{e:?}"))?;
    let requested_mode = req
        .mode
        .as_deref()
        .map(|m| m.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "escpos".to_string());
    let raw_label_mode = matches!(requested_mode.as_str(), "zpl" | "tspl" | "epl" | "cpcl");
    let bytes = if raw_label_mode {
        req.text.as_bytes().to_vec()
    } else {
        let mut stream = print_raw::EscPos::new();
        for line in req.text.lines() {
            stream = stream.text_line(line);
        }
        stream = stream.feed_lines(3);
        if req.cut {
            stream = stream.cut_partial();
        }
        stream.bytes().to_vec()
    };
    let mode = if raw_label_mode {
        requested_mode
    } else {
        "escpos".to_string()
    };
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
        mode: mode.clone(),
        paper: None,
        copies: copies.max(1),
        scale_policy: None,
        duplex: None,
        color: false,
        cut: if raw_label_mode { false } else { req.cut },
        tray: None,
        media_roll_width_mm: None,
        label_gap_mm: None,
        source_width_mm: None,
    };
    let id = enqueue_idempotent(&state.queue, &job, &mode, &bytes, &opts)?;
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
    check_license(&state)?;
    let copies = req.copies.unwrap_or(1).max(1);
    // Validate BEFORE decoding untrusted image bytes (see print_pdf).
    if !print_core::security::valid_idempotency_key(&req.idempotency_key) {
        return Err("bad_idempotency_key".to_string());
    }
    check_printer_pattern(&webview, &state, &req.printer_pattern)?;
    if req.report_name.len() > 512 {
        return Err("bad_report_name".to_string());
    }
    print_core::security::validate_payload_limits(req.bytes.len(), copies)
        .map_err(|e| format!("{e:?}"))?;
    let queue_arc = Arc::clone(&state.queue);
    let id = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        // Validate PNG signature + IHDR dimensions in O(1) at enqueue instead of
        // decoding every pixel twice (the worker decodes the PNG when printing).
        if req.bytes.len() < 24
            || &req.bytes[0..8] != b"\x89PNG\r\n\x1a\n"
            || &req.bytes[12..16] != b"IHDR"
        {
            return Err("invalid PNG header".to_string());
        }
        let png_w =
            u32::from_be_bytes([req.bytes[16], req.bytes[17], req.bytes[18], req.bytes[19]]);
        let png_h =
            u32::from_be_bytes([req.bytes[20], req.bytes[21], req.bytes[22], req.bytes[23]]);
        if png_w == 0 || png_h == 0 || png_w > 16_384 || png_h > 16_384 {
            return Err(format!("invalid PNG dimensions: {png_w}x{png_h}"));
        }
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
            duplex: None,
            color: false,
            cut: true,
            tray: None,
            media_roll_width_mm: req
                .media_roll_width_mm
                .filter(|w| w.is_finite() && (20.0..=500.0).contains(w)),
            label_gap_mm: req
                .label_gap_mm
                .filter(|g| g.is_finite() && (0.0..=100.0).contains(g)),
            source_width_mm,
        };
        enqueue_idempotent(&queue_arc, &job, "image-png", &req.bytes, &opts)
    })
    .await
    .map_err(|e| e.to_string())??;
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
    force: Option<bool>,
) -> Result<AppOverviewResponse, String> {
    check_origin(&webview, &state)?;
    let pdfium_ready = match PDFIUM_READY.get().copied() {
        Some(ready) => ready,
        None => tauri::async_runtime::spawn_blocking(pdfium_ready)
            .await
            .map_err(|e| e.to_string())?,
    };
    let (printers, default_printer) = cached_printers(force.unwrap_or(false)).await?;
    let origin = read_lock(&state.configured_origin).clone();
    let auto_reprint = *read_lock(&state.auto_reprint);
    let bundle = {
        let queue = lock_queue(&state.queue);
        build_bundle_from_queue(&queue).map_err(|e| e.to_string())?
    };
    let environment = read_lock(&state.environment).clone();
    Ok(AppOverviewResponse {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        odoo_origin: origin,
        locked_by_policy: state.locked_by_policy,
        auto_reprint,
        pdfium_ready,
        webview_version: webview_version_cached().to_string(),
        environment,
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
    force: Option<bool>,
) -> Result<Vec<PrinterInfo>, String> {
    check_origin(&webview, &state)?;
    let (printers, _default) = cached_printers(force.unwrap_or(false)).await?;
    Ok(printers)
}

#[tauri::command]
pub async fn printer_forms(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    printer: String,
) -> Result<Vec<Form>, String> {
    check_origin(&webview, &state)?;
    let trimmed = printer.trim().to_string();
    if trimmed.is_empty() || trimmed.len() > 512 {
        return Err("printer_required".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        print_win::paper_forms(&trimmed).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn printer_capabilities(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    printer: String,
) -> Result<print_core::PrinterCapabilities, String> {
    check_origin(&webview, &state)?;
    let trimmed = printer.trim().to_string();
    if trimmed.is_empty() || trimmed.len() > 512 {
        return Err("printer_required".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        print_win::printer_capabilities(&trimmed).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Boot-page reachability probe: TCP-connect to the configured
/// origin. Host/port come from print-core's security parser (the
/// one parser for this value — audit L5), and every resolved
/// address is tried under one shared 3 s deadline: a dual-stack
/// server whose first family is dead must not read as "down" when
/// the webview itself would connect via the other family.
#[tauri::command]
pub async fn probe_origin(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    check_local_only(&webview)?;
    let origin = read_lock(&state.configured_origin).clone();
    let Some(origin) = origin else {
        return Ok(false);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let Some((host, port)) = print_core::security::origin_host_port(&origin) else {
            return false;
        };
        let addrs = match (host.as_str(), port).to_socket_addrs() {
            Ok(a) => a,
            Err(_) => return false,
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        for sock in addrs {
            if is_blocked_metadata_ip(sock.ip()) {
                continue;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            if std::net::TcpStream::connect_timeout(&sock, remaining).is_ok() {
                return true;
            }
        }
        false
    })
    .await
    .map_err(|e| e.to_string())
}

/// Find a form by name in a capabilities report (case-insensitive,
/// as driver form names vary in casing between vendors).
fn find_caps_form(
    caps: &print_core::PrinterCapabilities,
    name: &str,
) -> Option<(String, f64, f64)> {
    caps.forms
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(name.trim()))
        .map(|f| (f.name.clone(), f.size.width_mm, f.size.height_mm))
}

/// The form a document test (or any automatic document job)
/// belongs on for this printer: its A4 form when it has one, else
/// Letter, else its largest sheet, else its roll width as a
/// 297 mm strip. Nothing here assumes A4 exists — the choice is
/// derived from what the driver actually reports.
fn auto_document_form(caps: &print_core::PrinterCapabilities) -> Option<(String, f64, f64)> {
    let sheets: Vec<&print_core::Form> = caps.forms.iter().filter(|f| !f.roll).collect();
    for want in ["A4", "Letter"] {
        if let Some(f) = sheets.iter().find(|f| f.name.eq_ignore_ascii_case(want)) {
            return Some((f.name.clone(), f.size.width_mm, f.size.height_mm));
        }
    }
    sheets
        .iter()
        .max_by(|a, b| {
            (a.size.width_mm * a.size.height_mm)
                .partial_cmp(&(b.size.width_mm * b.size.height_mm))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|f| (f.name.clone(), f.size.width_mm, f.size.height_mm))
        .or_else(|| {
            caps.forms
                .iter()
                .find(|f| f.roll)
                .map(|f| (f.name.clone(), f.size.width_mm, 297.0))
        })
}

/// Document options for a calibration/test print, grouped the
/// same way the print commands group theirs.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestPrintOptions {
    pub tray: Option<String>,
    pub duplex: Option<bool>,
    pub color: Option<bool>,
}

#[tauri::command]
pub async fn run_test_print(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    printer: String,
    kind: String,
    paper: Option<String>,
    options: Option<TestPrintOptions>,
) -> Result<String, String> {
    // Test prints are a Control Center feature: the Odoo page must
    // never fire raw calibrations at local printers (audit B1).
    check_local_only(&webview)?;
    if printer.trim().is_empty() || printer.len() > 512 {
        return Err("printer_required".to_string());
    }
    let saved_paper = read_lock(&state.printer_settings)
        .get(printer.trim())
        .and_then(|s| s.paper.clone());
    let queue_arc = Arc::clone(&state.queue);
    let id = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let options = options.unwrap_or_default();
        let tray = options.tray;
        let duplex = options.duplex;
        let color = options.color;
    // Raw calibration pages (ESC/POS / ZPL / TSPL) sent to a virtual
    // PDF/XPS writer produce a corrupt file instead of a page; refuse
    // up front with the reason instead of enqueueing a doomed job.
    let raw_kind = matches!(
        kind.as_str(),
        "thermal-80mm" | "thermal-58mm" | "escpos" | "zpl-label" | "zpl" | "tspl-label" | "tspl" | "epl-label" | "epl" | "cpcl-label" | "cpcl"
    );
    if raw_kind && !printer.contains("://") {
        // Fail closed: a raw calibration is refused whenever the
        // target's kind cannot be positively verified as physical.
        let discovered = print_win::discover_printers().unwrap_or_default();
        let found = discovered.iter().find(|p| p.name == printer.trim());
        let verified_physical = matches!(found, Some(f) if f.kind != PrinterKind::Virtual);
        if !verified_physical {
            return Err(
                "Raw calibration (ESC/POS, ZPL, TSPL) requires a verified physical printer. Select a physical printer from the list."
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
                "epl-label" | "epl" => "epl",
                "cpcl-label" | "cpcl" => "cpcl",
                _ => "",
            };
            if !wanted.is_empty() && f.protocol != wanted {
                return Err(format!(
                    "Calibration language {} does not match \"{}\" ({}, {}). Select the calibration that matches this printer.",
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
                    duplex: None,
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
                    duplex: None,
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
                .text(40, 20, "Itbaa TSPL Calibration")
                .print(1, 1);
            (
                "tspl",
                tspl,
                JobOptions {
                    mode: "tspl".to_string(),
                    paper: Some(paper.unwrap_or_else(|| "100x150mm".to_string())),
                    copies: 1,
                    scale_policy: Some("custom_allowed".to_string()),
                    duplex: None,
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
        "epl-label" | "epl" => {
            let bm = print_render::calibration::calibration_thermal_80mm();
            let epl = print_raw::epl::epl_label(&bm);
            (
                "epl",
                epl,
                JobOptions {
                    mode: "epl".to_string(),
                    paper: Some(paper.unwrap_or_else(|| "100x150mm".to_string())),
                    copies: 1,
                    scale_policy: Some("custom_allowed".to_string()),
                    duplex: None,
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
        "cpcl-label" | "cpcl" => {
            let bm = print_render::calibration::calibration_thermal_80mm();
            let cpcl = print_raw::cpcl::bitmap_label(&bm, 203, 1)
                .map_err(|e| e.to_string())?;
            (
                "cpcl",
                cpcl,
                JobOptions {
                    mode: "cpcl".to_string(),
                    paper: Some(paper.unwrap_or_else(|| "100x150mm".to_string())),
                    copies: 1,
                    scale_policy: Some("custom_allowed".to_string()),
                    duplex: None,
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
            // The test page is generated AT the size it will print
            // on — the form the user chose in the lab (by name,
            // resolved against the printer's driver capabilities),
            // else the paper saved for this printer in the Control
            // Center, else the printer's automatic document form —
            // never a hardcoded A4 (owner directive, 2026-11-05:
            // test prints ignored every chosen size and always
            // came out A4). The job carries the form's real name so
            // the driver selects exactly that form, and the lab's
            // tray/duplex/color ride along.
            let caps = print_win::printer_capabilities(printer.trim()).ok();
            let chosen = paper
                .clone()
                .filter(|n| !n.trim().is_empty() && !n.eq_ignore_ascii_case("auto"))
                .or(saved_paper);
            let form = chosen
                .as_deref()
                .and_then(|name| caps.as_ref().and_then(|c| find_caps_form(c, name)))
                .or_else(|| caps.as_ref().and_then(auto_document_form))
                .unwrap_or_else(|| ("A4".to_string(), 210.0, 297.0));
            let pdf = print_render::calibration::calibration_pdf_sized(form.1, form.2);
            (
                "pdf",
                pdf,
                JobOptions {
                    mode: "gdi".to_string(),
                    paper: Some(form.0),
                    copies: 1,
                    scale_policy: Some("custom_allowed".to_string()),
                    duplex,
                    color: color.unwrap_or(false),
                    cut: false,
                    tray: tray.filter(|t| !t.trim().is_empty()),
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
        enqueue_idempotent(&queue_arc, &job, payload_kind, &payload_bytes, &opts)
    })
    .await
    .map_err(|e| e.to_string())??;
    state.notify_worker();
    Ok(id)
}

#[tauri::command]
pub async fn retry_job_cmd(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), String> {
    check_local_only(&webview)?;
    let queue_arc = Arc::clone(&state.queue);
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut queue = lock_queue(&queue_arc);
        queue.retry_job(&job_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    state.notify_worker();
    Ok(())
}

#[tauri::command]
pub async fn cancel_job_cmd(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), String> {
    check_local_only(&webview)?;
    let queue_arc = Arc::clone(&state.queue);
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut queue = lock_queue(&queue_arc);
        queue.cancel_job(&job_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn clear_finished_cmd(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    check_local_only(&webview)?;
    let queue_arc = Arc::clone(&state.queue);
    tauri::async_runtime::spawn_blocking(move || -> Result<u64, String> {
        let mut queue = lock_queue(&queue_arc);
        queue.clear_finished().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Strip scheme, credentials, host/port, query string, and fragment from any
/// URI embedded in diagnostic error strings, preserving only the path.
fn redact_urls_in_text(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut pos = 0;
    while let Some(rel) = input[pos..].find("://") {
        let sep = pos + rel;
        let mut scheme_start = sep;
        while scheme_start > pos {
            let b = bytes[scheme_start - 1];
            if b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.') {
                scheme_start -= 1;
            } else {
                break;
            }
        }
        if scheme_start == sep || !bytes[scheme_start].is_ascii_alphabetic() {
            out.push_str(&input[pos..sep + 3]);
            pos = sep + 3;
            continue;
        }
        out.push_str(&input[pos..scheme_start]);
        let after_scheme = sep + 3;
        let url_end = input[after_scheme..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>' | ']'))
            .map(|i| after_scheme + i)
            .unwrap_or(input.len());
        let rest = &input[after_scheme..url_end];
        let cut = rest.find(['?', '#']).unwrap_or(rest.len());
        let no_query = &rest[..cut];
        out.push_str("[redacted-origin]");
        if let Some(slash) = no_query.find('/') {
            out.push_str(&no_query[slash..]);
        }
        pos = url_end;
    }
    out.push_str(&input[pos..]);
    out
}

/// Allowlist and redact environment probe fields before exporting diagnostics
/// so `href`, server origin, query tokens, and unexpected remote keys never leak.
fn redact_environment_for_diagnostics(env: &serde_json::Value) -> serde_json::Value {
    let Some(obj) = env.as_object() else {
        return serde_json::json!({});
    };
    let mut clean = serde_json::Map::new();
    for key in [
        "secureContext",
        "online",
        "es2022",
        "structuredClone",
        "randomUUID",
        "localStorage",
        "sessionStorage",
        "cookies",
        "webSocket",
    ] {
        if let Some(v) = obj.get(key).and_then(|v| v.as_bool()) {
            clean.insert(key.to_string(), serde_json::Value::Bool(v));
        }
    }
    if let Some(ua) = obj.get("userAgent").and_then(|v| v.as_str()) {
        clean.insert(
            "userAgent".to_string(),
            serde_json::Value::String(ua.chars().take(300).collect()),
        );
    }
    if let Some(path) = obj.get("pathname").and_then(|v| v.as_str()) {
        let stripped = path.split(['?', '#']).next().unwrap_or("/");
        clean.insert(
            "pathname".to_string(),
            serde_json::Value::String(stripped.chars().take(200).collect()),
        );
    }
    if let Some(errs) = obj.get("errors").and_then(|v| v.as_array()) {
        let redacted: Vec<serde_json::Value> = errs
            .iter()
            .take(10)
            .filter_map(|e| e.as_str())
            .map(|s| {
                let truncated: String = s.chars().take(300).collect();
                serde_json::Value::String(redact_urls_in_text(&truncated))
            })
            .collect();
        clean.insert("errors".to_string(), serde_json::Value::Array(redacted));
    }
    serde_json::Value::Object(clean)
}

fn insert_license_diagnostics(
    value: &mut serde_json::Value,
    status: &print_core::license::LicenseStatus,
    now_utc: u64,
) {
    let days_remaining = if status.valid && !status.perpetual {
        status
            .exp
            .filter(|&exp| exp > 0)
            .map(|exp| exp.saturating_sub(now_utc) / 86_400)
    } else {
        None
    };
    value["license_valid"] = serde_json::json!(status.valid);
    value["license_code"] = serde_json::json!(status.code);
    value["license_tier"] = serde_json::json!(status.tier);
    value["license_perpetual"] = serde_json::json!(status.perpetual);
    value["license_days_remaining"] = serde_json::json!(days_remaining);
}

#[tauri::command]
pub async fn export_diagnostics(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<String, String> {
    check_local_only(&webview)?;
    let pdfium = match PDFIUM_READY.get().copied() {
        Some(ready) => ready,
        None => tauri::async_runtime::spawn_blocking(pdfium_ready)
            .await
            .map_err(|e| e.to_string())?,
    };
    let bundle = {
        let queue = lock_queue(&state.queue);
        build_bundle_from_queue(&queue).map_err(|e| e.to_string())?
    };
    let mut value: serde_json::Value =
        serde_json::to_value(&bundle).unwrap_or_else(|_| serde_json::json!({}));
    // The bundle is built by print-core and stamps its own crate
    // version; report the desktop app's version instead.
    value["app_version"] = serde_json::json!(env!("CARGO_PKG_VERSION"));
    value["webview_version"] = serde_json::json!(webview_version_cached());
    value["arch"] = serde_json::json!(std::env::consts::ARCH);
    value["uptime_secs"] = serde_json::json!(state.started_at.elapsed().as_secs());
    value["open_tab_count"] = serde_json::json!(read_lock(&state.tabs).order.len());
    value["pdfium_ready"] = serde_json::json!(pdfium);
    value["locked_by_policy"] = serde_json::json!(state.locked_by_policy);
    value["auto_reprint"] = serde_json::json!(*read_lock(&state.auto_reprint));
    let origin_opt = read_lock(&state.configured_origin).clone();
    value["origin_configured"] = serde_json::json!(origin_opt.is_some());
    let lic_now = print_core::license::now_utc_secs();
    let lic_key = read_lock(&state.license_key).clone();
    let lic_last_seen = *read_lock(&state.last_seen_utc);
    let lic_res = print_core::license::verify_license(
        lic_key.as_deref(),
        origin_opt.as_deref().unwrap_or(""),
        lic_now,
        lic_last_seen,
    );
    let lic_status = print_core::license::LicenseStatus::from_result(&lic_res);
    insert_license_diagnostics(&mut value, &lic_status, lic_now);
    if let Some(env) = read_lock(&state.environment).clone() {
        value["environment"] = redact_environment_for_diagnostics(&env);
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
    let origin = read_lock(&state.configured_origin).clone();
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
    pub startup_url: Option<String>,
}

#[tauri::command]
pub fn app_identity(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AppIdentity, String> {
    check_local_only(&webview)?;
    let origin = read_lock(&state.configured_origin).clone();
    let startup_url = origin
        .as_deref()
        .and_then(|o| config::load_last_url(&state.data_dir, o));
    Ok(AppIdentity {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        odoo_origin: origin,
        startup_url,
    })
}

/// Create a tab for `url` and make it active. Shared by the
/// tab_open command, the POS navigation rule and window.open
/// handling. Async: creating a webview from a synchronous command
/// can deadlock on Windows.
const MAX_TABS: usize = 24;

pub async fn open_tab_internal(
    app: &tauri::AppHandle,
    state: &AppState,
    url: tauri::Url,
) -> Result<String, String> {
    let label = {
        let mut tabs = write_lock(&state.tabs);
        if tabs.order.len() >= MAX_TABS {
            return Err("too_many_tabs".to_string());
        }
        tabs.next_id += 1;
        format!("tab-{}", tabs.next_id)
    };
    let web_url = if is_local_app_url(&url) {
        let path = url.path().trim_start_matches('/');
        tauri::WebviewUrl::App(path.into())
    } else {
        tauri::WebviewUrl::External(url.clone())
    };
    crate::create_tab_webview(app, &label, web_url)?;
    {
        let mut tabs = write_lock(&state.tabs);
        tabs.register(&label, url.as_str());
    }
    crate::activate_tab(app, &label);
    Ok(label)
}

/// Open a URL in a new tab. Callable from the configured Odoo
/// origin (allow-app-shell): the URL must belong to that origin —
/// validated here in Rust, never trusted from the page.
#[tauri::command]
pub async fn tab_open(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    url: String,
) -> Result<String, String> {
    check_origin(&webview, &state)?;
    // An empty URL is the tab bar's "new tab" action: a fresh tab
    // on the configured origin (the Control Center when none is
    // configured yet).
    let raw = url.trim();
    if raw.len() > 8192 {
        return Err("bad_tab_url".to_string());
    }
    if raw.is_empty() {
        let origin = read_lock(&state.configured_origin).clone();
        let parsed = match origin {
            Some(origin) => tauri::Url::parse(&origin).map_err(|_| "bad_tab_url".to_string())?,
            None => local_app_url("index.html")?,
        };
        return open_tab_internal(&app, &state, parsed).await;
    }
    let parsed = if raw.starts_with('/') {
        let configured = read_lock(&state.configured_origin)
            .clone()
            .ok_or_else(|| "no_origin_configured".to_string())?;
        let base = tauri::Url::parse(&configured).map_err(|_| "bad_tab_url".to_string())?;
        base.join(raw).map_err(|_| "bad_tab_url".to_string())?
    } else {
        tauri::Url::parse(raw).map_err(|_| "bad_tab_url".to_string())?
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("bad_tab_url".to_string());
    }
    if is_local_app_url(&parsed) {
        check_local_only(&webview)?;
    } else {
        let configured = read_lock(&state.configured_origin)
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
    let tabs = read_lock(&state.tabs);
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
        let tabs = read_lock(&state.tabs);
        if !tabs.order.iter().any(|l| l == &label) {
            return Err("unknown_tab".to_string());
        }
    }
    crate::activate_tab(&app, &label);
    Ok(())
}

/// Reorder a tab (tab bar only, drag-and-drop): the tab bar owns
/// the strip's arrangement exactly like a browser's — the user
/// drags a tab to its new position and the strip follows.
#[tauri::command]
pub fn tab_move(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    label: String,
    to_index: usize,
) -> Result<(), String> {
    check_local_only(&webview)?;
    let mut tabs = write_lock(&state.tabs);
    if tabs.move_to(&label, to_index) {
        Ok(())
    } else {
        Err("unknown_tab".to_string())
    }
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
    let is_last = {
        let tabs = read_lock(&state.tabs);
        if !tabs.order.iter().any(|l| l == &label) {
            return Err("unknown_tab".to_string());
        }
        tabs.order.len() == 1
    };
    if is_last {
        let fallback_url = if read_lock(&state.configured_origin).is_some() {
            local_app_url("boot.html")?
        } else {
            local_app_url("index.html")?
        };
        open_tab_internal(&app, &state, fallback_url).await?;
    }
    let (was_active, neighbor) = {
        let mut tabs = write_lock(&state.tabs);
        let idx = tabs.order.iter().position(|l| l == &label);
        let Some(idx) = idx else {
            return Ok(());
        };
        let was_active = tabs.active.as_deref() == Some(label.as_str());
        tabs.order.remove(idx);
        tabs.urls.remove(&label);
        tabs.pos_tabs.remove(&label);
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
    if let Some(next) = neighbor {
        if was_active {
            crate::activate_tab(&app, &next);
        }
    } else {
        let fallback_url = if read_lock(&state.configured_origin).is_some() {
            local_app_url("boot.html")?
        } else {
            local_app_url("index.html")?
        };
        open_tab_internal(&app, &state, fallback_url).await?;
    }
    Ok(())
}

/// The active content webview, for the navigation commands.
fn active_tab_webview(app: &tauri::AppHandle, state: &AppState) -> Result<tauri::Webview, String> {
    let label = {
        let tabs = read_lock(&state.tabs);
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
    let parsed = serde_json::from_str::<serde_json::Value>(&report).ok();
    {
        let mut guard = write_lock(&state.environment);
        *guard = parsed;
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
        let tabs = read_lock(&state.tabs);
        tabs.order.clone()
    };
    for label in &labels {
        if let Some(wv) = app.get_webview(label) {
            wv.clear_all_browsing_data().map_err(|e| e.to_string())?;
        }
    }
    config::clear_last_url(&state.data_dir);
    let origin = read_lock(&state.configured_origin).clone();
    if let Some(origin) = origin {
        let url = origin.parse::<tauri::Url>().map_err(|e| e.to_string())?;
        let snapshot = {
            let tabs = read_lock(&state.tabs);
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
pub async fn save_origin_cmd(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    origin: String,
    auto_reprint: Option<bool>,
) -> Result<String, String> {
    check_local_only(&webview)?;
    if state.locked_by_policy {
        return Err("locked_by_policy".to_string());
    }
    let current_auto = *read_lock(&state.auto_reprint);
    let next_auto = auto_reprint.unwrap_or(current_auto);
    let config_dir = state.config_dir.clone();
    let validated = tauri::async_runtime::spawn_blocking(move || {
        config::save_origin(&config_dir, &origin, next_auto)
    })
    .await
    .map_err(|e| e.to_string())??;
    let prev_origin = {
        let mut guard = write_lock(&state.configured_origin);
        guard.replace(validated.clone())
    };
    {
        let mut auto_guard = write_lock(&state.auto_reprint);
        *auto_guard = next_auto;
    }
    // Keep open remote tabs synchronized when the Odoo server URL changes so they
    // never get stranded on a stale origin that fails IPC origin checks.
    if prev_origin.as_deref() != Some(validated.as_str()) {
        let data_dir = state.data_dir.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            config::clear_last_url(&data_dir);
        })
        .await;
        if let Ok(target_url) = validated.parse::<tauri::Url>() {
            let snapshot = read_lock(&state.tabs).snapshot();
            for tab in snapshot.iter().filter(|t| !t.is_local) {
                if let Some(wv) = app.get_webview(&tab.label) {
                    let _ = wv.navigate(target_url.clone());
                }
            }
        }
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
    Ok(read_lock(&state.printer_settings).clone())
}

#[tauri::command]
pub async fn set_printer_settings(
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
    let printer_settings = Arc::clone(&state.printer_settings);
    let config_dir = state.config_dir.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<config::PrinterSettingsMap, String> {
        let mut guard = write_lock(&printer_settings);
        let mut map = config::load_printer_settings_checked(&config_dir)?;
        if settings == config::PrinterSettings::default() {
            map.remove(&name);
        } else {
            map.insert(name, settings);
        }
        config::save_printer_settings(&config_dir, &map)?;
        *guard = map.clone();
        Ok(map)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Open one of the app's built-in local pages in its own tab
/// (owner brief, 2026-11-05): if a tab already shows the page it
/// is focused; opening never replaces the page the user was
/// working on.
async fn open_local_page(
    app: &tauri::AppHandle,
    state: &AppState,
    file: &str,
) -> Result<(), String> {
    let existing = {
        let tabs = read_lock(&state.tabs);
        tabs.order
            .iter()
            .find(|label| {
                tabs.url_of(label)
                    .and_then(|u| tauri::Url::parse(u).ok())
                    .map(|u| {
                        is_local_app_url(&u)
                            && (u.path().ends_with(file)
                                || (file == "index.html" && u.path() == "/"))
                    })
                    .unwrap_or(false)
            })
            .cloned()
    };
    if let Some(label) = existing {
        crate::activate_tab(app, &label);
        return Ok(());
    }
    let local_url = local_app_url(file)?;
    open_tab_internal(app, state, local_url).await?;
    Ok(())
}

#[tauri::command]
pub async fn open_control_center(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_origin(&webview, &state)?;
    open_local_page(&app, &state, "index.html").await
}

/// The browser-style Settings page (About + contact), opened from
/// the tab bar next to the Control Center (owner, 2026-11-05).
#[tauri::command]
pub async fn open_settings(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_origin(&webview, &state)?;
    open_local_page(&app, &state, "settings.html").await
}

#[tauri::command]
pub async fn open_odoo_origin(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_local_only(&webview)?;
    let origin = read_lock(&state.configured_origin)
        .clone()
        .ok_or_else(|| "no_origin_configured".to_string())?;
    let url: tauri::Url = origin.parse().map_err(|_| "bad_origin_url".to_string())?;
    // Tabbed browser: prefer an existing remote tab (switch back
    // to Odoo without losing anything), navigate the calling tab
    // when it is the only candidate (the boot page flow), and give
    // the tab bar a fresh tab.
    let caller = webview.label().to_string();
    let (existing, caller_is_tab) = {
        let tabs = read_lock(&state.tabs);
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
        return Ok(());
    }
    if caller_is_tab {
        webview.navigate(url).map_err(|e| e.to_string())?;
        return Ok(());
    }
    open_tab_internal(&app, &state, url).await?;
    Ok(())
}

// Background print worker and hardware dispatch.

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

fn is_blocked_metadata_v4(o: [u8; 4]) -> bool {
    matches!(
        o,
        [169, 254, 169, 254] | [169, 254, 170, 2] | [100, 100, 100, 200]
    )
}

/// Block cloud instance metadata endpoints (AWS/GCP/Azure/ECS/Alibaba IPv4,
/// AWS IPv6 IMDS `fd00:ec2::254`, and IPv4-mapped / IPv4-compatible / NAT64
/// IPv6 encodings) against SSRF via printer URIs, origin config, or probes.
pub(crate) fn is_blocked_metadata_ip(ip: std::net::IpAddr) -> bool {
    const AWS_IMDS_V6: [u8; 16] = [
        0xfd, 0x00, 0x0e, 0xc2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0x54,
    ];
    const NAT64_PREFIX: [u8; 12] = [0x00, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0];
    match ip {
        std::net::IpAddr::V4(v4) => is_blocked_metadata_v4(v4.octets()),
        std::net::IpAddr::V6(v6) => {
            let o = v6.octets();
            if o == AWS_IMDS_V6 {
                return true;
            }
            let v4_tail = [o[12], o[13], o[14], o[15]];
            let embedded = (o[..10] == [0u8; 10] && o[10] == 0xff && o[11] == 0xff)
                || o[..12] == [0u8; 12]
                || o[..12] == NAT64_PREFIX;
            embedded && is_blocked_metadata_v4(v4_tail)
        }
    }
}

pub(crate) fn is_blocked_metadata_host(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    matches!(
        h.as_str(),
        "metadata.google.internal" | "metadata.goog" | "instance-data"
    )
}

struct NativePrintBackend {
    db_path: String,
    printer_settings: Arc<RwLock<config::PrinterSettingsMap>>,
    configured_origin: Arc<RwLock<Option<String>>>,
    license_key: Arc<RwLock<Option<String>>>,
    last_seen_utc: Arc<RwLock<u64>>,
    read_queue: Option<Queue>,
}

impl NativePrintBackend {
    /// Reuse a persistent read handle across print jobs so each job does not
    /// reopen SQLite and re-run WAL pragmas; drop the handle on DB errors so
    /// the next attempt reopens cleanly.
    fn load_job_data(
        &mut self,
        job_id: &str,
    ) -> Result<(String, Vec<u8>, JobOptions), BackendError> {
        if self.read_queue.is_none() {
            let q = Queue::open(&self.db_path)
                .map_err(|e| BackendError::retryable("QUEUE_DB_ERROR", &e.to_string()))?;
            self.read_queue = Some(q);
        }
        let payload = {
            let q = self.read_queue.as_ref().ok_or_else(|| {
                BackendError::retryable("QUEUE_DB_ERROR", "read queue unavailable")
            })?;
            q.payload(job_id)
        };
        let (kind, data) = match payload {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                return Err(BackendError::fatal(
                    "MISSING_PAYLOAD",
                    "job payload not found",
                ))
            }
            Err(e) => {
                if matches!(e, print_core::QueueError::Db(_)) {
                    self.read_queue = None;
                }
                return Err(BackendError::retryable("QUEUE_DB_ERROR", &e.to_string()));
            }
        };
        let opts = {
            let q = self.read_queue.as_ref().ok_or_else(|| {
                BackendError::retryable("QUEUE_DB_ERROR", "read queue unavailable")
            })?;
            q.options(job_id)
        };
        let opts = match opts {
            Ok(o) => o,
            Err(e) => {
                if matches!(e, print_core::QueueError::Db(_)) {
                    self.read_queue = None;
                }
                return Err(BackendError::retryable(
                    "QUEUE_DB_ERROR",
                    &format!("stored job options unreadable: {e}"),
                ));
            }
        };
        Ok((kind, data, opts))
    }

    /// Scale a 203-dpi print-head profile width (see
    /// `receipt_dots_for_paper`) to the device's reported render
    /// resolution. A 300-dpi head prints each ESC/POS dot smaller,
    /// so the unscaled dot count would come out physically narrow;
    /// >300 dpi (e.g. 600 dpi from Windows Generic / Text Only) is a
    /// GDI pseudo-resolution and stays at 203 dpi. Always align to a
    /// whole byte (multiple of 8 dots) within 256..=2048 so GS v 0
    /// raster encoding never rejects a scaled width.
    fn scale_head_dots(dots: u32, caps: &print_core::printer::PrinterCapabilities) -> u32 {
        let dpi = print_core::printer::preferred_raw_render_dpi(&caps.resolutions_dpi);
        let scaled = if dpi > 300 {
            dots
        } else {
            ((dots as u64 * dpi as u64 + 101) / 203) as u32
        };
        (((scaled + 4) / 8) * 8).clamp(256, 2048)
    }

    fn is_office_sheet_or_envelope(name: &str) -> bool {
        let lower = name.trim().to_ascii_lowercase();
        [
            "letter",
            "legal",
            "tabloid",
            "ledger",
            "folio",
            "executive",
            "statement",
            "fanfold",
            "envelope",
            "monarch",
            "a3",
            "a4",
            "a5",
            "b4",
            "b5",
            "b6",
        ]
        .iter()
        .any(|tok| lower.contains(tok))
    }

    fn map_image_error(err: print_render::ImageError) -> BackendError {
        let code = match err {
            print_render::ImageError::DimensionsTooLarge(_, _) => "IMAGE_DIMENSIONS_ERROR",
            _ => "IMAGE_DECODE_ERROR",
        };
        BackendError::fatal(code, &err.to_string())
    }

    /// Apply Control Center per-printer defaults, scoped so a default
    /// only fills fields that describe the job's own payload:
    /// paper/duplex/color are DOCUMENT settings and apply to PDF
    /// jobs only — a sheet paper saved for a printer must never
    /// become a receipt's paper override. The tray is a transport
    /// setting every GDI/IPP arm consumes and raw arms ignore, so
    /// it fills for any kind. A job's own values always win.
    fn apply_printer_defaults(
        opts: &mut JobOptions,
        kind: &str,
        defaults: &config::PrinterSettings,
    ) {
        if opts.tray.is_none() {
            opts.tray = defaults.tray.clone();
        }
        if kind == "pdf" {
            if opts.paper.is_none() {
                opts.paper = defaults.paper.clone();
            }
            if opts.duplex.is_none() {
                if let Some(d) = defaults.duplex {
                    opts.duplex = Some(d);
                }
            }
            if let Some(c) = defaults.color {
                opts.color = c;
            }
        }
    }

    /// Resolve ESC/POS raster width (dots) for a receipt job from the
    /// best available width evidence (paper override, driver roll form,
    /// or rendered receipt width).
    fn receipt_width_dots(
        printer: &str,
        paper_override: Option<&str>,
        job_media_mm: Option<f64>,
        source_width_mm: Option<f64>,
    ) -> Result<u32, BackendError> {
        // An override naming something that is not a receipt paper
        // (a sheet form name like "A4" that leaked into a receipt
        // job from a document-oriented setting) is IGNORED, never
        // fatal: the evidence ladder below still sizes the receipt
        // from real media evidence (2026-11-05 field incident).
        if let Some(dots) = paper_override.and_then(print_render::receipt_dots_for_paper) {
            let paper = paper_override.unwrap_or_default();
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
                        eprintln!(
                            "Itbaa: paper override {paper:?} differs from driver roll forms {rolls:?} mm; honoring explicit override"
                        );
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
                        .filter(|form| !Self::is_office_sheet_or_envelope(&form.name))
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
        // Pragmatic default: standard 80 mm thermal head (576 dots) when the driver
        // exposes no usable form dimensions and the job omitted width metadata.
        eprintln!(
            "Itbaa: no roll width reported for {printer:?}; falling back to 80 mm (576 dots)"
        );
        Ok(576)
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

    // Metadata host/IP checks live at module level so config.rs shares them.

    fn reject_metadata_resolution(host: &str, port: u16) -> Result<(), BackendError> {
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if is_blocked_metadata_ip(ip) {
                return Err(BackendError::fatal(
                    "BAD_PRINTER_ADDRESS",
                    "cloud metadata address is not allowed",
                ));
            }
            return Ok(());
        }
        let addrs = (host, port).to_socket_addrs().map_err(|e| {
            BackendError::retryable(
                "DNS_RESOLUTION_FAILED",
                &format!("cannot resolve printer host: {e}"),
            )
        })?;
        let mut resolved = false;
        for sock in addrs {
            resolved = true;
            if is_blocked_metadata_ip(sock.ip()) {
                return Err(BackendError::fatal(
                    "BAD_PRINTER_ADDRESS",
                    "cloud metadata address is not allowed",
                ));
            }
        }
        if !resolved {
            return Err(BackendError::retryable(
                "DNS_RESOLUTION_FAILED",
                "printer host resolved to no addresses",
            ));
        }
        Ok(())
    }

    fn is_valid_hostname(host: &str) -> bool {
        if host.is_empty() || host.len() > 253 {
            return false;
        }
        let trimmed = host.strip_suffix('.').unwrap_or(host);
        if trimmed.is_empty() || !trimmed.bytes().any(|b| b.is_ascii_alphabetic()) {
            return false;
        }
        trimmed.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    }

    fn repeat_raw_copies(bytes: &[u8], copies: u16) -> Result<Vec<u8>, BackendError> {
        let count = copies.max(1) as usize;
        let total = bytes
            .len()
            .checked_mul(count)
            .filter(|&n| n <= MAX_RAW_EXPANDED_BYTES)
            .ok_or_else(|| {
                BackendError::fatal(
                    "PAYLOAD_TOO_LARGE",
                    "expanded raw print payload exceeds 100 MiB limit",
                )
            })?;
        let mut out = Vec::with_capacity(total);
        for _ in 0..count {
            out.extend_from_slice(bytes);
        }
        Ok(out)
    }

    fn parse_host_port(
        spec: &str,
        default_port: u16,
    ) -> Result<(String, u16, String), BackendError> {
        let spec_trimmed = spec.trim();
        let lower = spec_trimmed.to_ascii_lowercase();
        let rest = ["tcp://", "raw://", "ipp://", "http://", "https://"]
            .iter()
            .find_map(|p| lower.starts_with(p).then(|| &spec_trimmed[p.len()..]))
            .unwrap_or(spec_trimmed);
        let (host_port, path) = match rest.split_once('/') {
            Some((hp, p)) => (hp, format!("/{p}")),
            None => (rest, "/ipp/print".to_string()),
        };
        if path.bytes().any(|b| b <= 0x20 || b == 0x7f) {
            return Err(BackendError::fatal(
                "BAD_PRINTER_ADDRESS",
                "printer path contains invalid whitespace or control characters",
            ));
        }
        let (host, port, is_ipv6) = if let Some(after_open) = host_port.strip_prefix('[') {
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
            (ipv6.to_string(), port, true)
        } else {
            match host_port.rsplit_once(':') {
                Some((h, p)) => {
                    let parsed = p.parse::<u16>().map_err(|_| {
                        BackendError::fatal(
                            "BAD_PRINTER_ADDRESS",
                            "invalid port in printer address",
                        )
                    })?;
                    (h.to_string(), parsed, false)
                }
                None => (host_port.to_string(), default_port, false),
            }
        };
        if host.is_empty() {
            return Err(BackendError::fatal(
                "BAD_PRINTER_ADDRESS",
                "empty printer host",
            ));
        }
        if port == 0 {
            return Err(BackendError::fatal(
                "BAD_PRINTER_ADDRESS",
                "printer port must be 1..=65535",
            ));
        }
        if is_ipv6 {
            let v6 = host.parse::<std::net::Ipv6Addr>().map_err(|_| {
                BackendError::fatal("BAD_PRINTER_ADDRESS", "invalid IPv6 printer host")
            })?;
            if is_blocked_metadata_ip(std::net::IpAddr::V6(v6)) {
                return Err(BackendError::fatal(
                    "BAD_PRINTER_ADDRESS",
                    "cloud metadata address is not allowed",
                ));
            }
        } else if let Ok(v4) = host.parse::<std::net::Ipv4Addr>() {
            if is_blocked_metadata_ip(std::net::IpAddr::V4(v4)) {
                return Err(BackendError::fatal(
                    "BAD_PRINTER_ADDRESS",
                    "cloud metadata address is not allowed",
                ));
            }
        } else if !Self::is_valid_hostname(&host) {
            return Err(BackendError::fatal(
                "BAD_PRINTER_ADDRESS",
                "invalid printer host",
            ));
        } else if is_blocked_metadata_host(&host) {
            return Err(BackendError::fatal(
                "BAD_PRINTER_ADDRESS",
                "cloud metadata address is not allowed",
            ));
        }
        Ok((host, port, path))
    }

    /// Configured label gap in millimetres; the printer's 3 mm
    /// default when the job carries none. Negative gaps are
    /// clamped to 0 (continuous media).
    fn label_gap_mm(gap: Option<f64>) -> f64 {
        gap.filter(|g| g.is_finite())
            .map(|g| g.clamp(0.0, 100.0))
            .unwrap_or(3.0)
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
        if count > print_core::security::MAX_PAGES as usize {
            return Err(BackendError::fatal(
                "TOO_MANY_PAGES",
                "PDF exceeds maximum page limit",
            ));
        }
        match mode {
            "escpos" => {
                // Default PDF reports on Generic/Text or raw-TCP thermal printers to 80 mm (576 dots)
                // when the driver reports no roll media instead of failing with AUTO_MEDIA_UNAVAILABLE.
                let target_w = target_width_dots
                    .or_else(|| paper.and_then(print_render::receipt_dots_for_paper))
                    .unwrap_or(576);
                let mut stream = print_raw::EscPos::new();
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
                Self::repeat_raw_copies(stream.bytes(), copies)
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
            "epl" | "cpcl" => {
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
                    if mode == "cpcl" {
                        let chunk = print_raw::cpcl::bitmap_label(
                            &bm,
                            render_dpi.clamp(72, u16::MAX as u32) as u16,
                            copies.max(1),
                        )
                        .map_err(|e| BackendError::fatal("CPCL_ENCODE_ERROR", &e.to_string()))?;
                        out.extend_from_slice(&chunk);
                    } else {
                        out.extend_from_slice(&print_raw::epl::epl_label_copies_with_gap(
                            &bm,
                            copies,
                            Self::label_gap_dots(label_gap_mm, render_dpi),
                        ));
                    }
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
                let canvas = print_render::decode_png(png_bytes).map_err(Self::map_image_error)?;
                let mono = canvas.to_mono(print_render::image::RECEIPT_THRESHOLD);
                let mono = match label_box {
                    Some((w, h)) => print_render::bitmap::fit_mono_to_box(&mono, w, h),
                    None => mono,
                };
                Ok(print_raw::zpl_label_with_options(&mono, 0, 0, copies).into_bytes())
            }
            "tspl" => {
                let canvas = print_render::decode_png(png_bytes).map_err(Self::map_image_error)?;
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
            "epl" | "cpcl" => {
                let canvas = print_render::decode_png(png_bytes).map_err(Self::map_image_error)?;
                let mono = canvas.to_mono(print_render::image::RECEIPT_THRESHOLD);
                let mono = match label_box {
                    Some((w, h)) => print_render::bitmap::fit_mono_to_box(&mono, w, h),
                    None => mono,
                };
                if mode == "cpcl" {
                    print_raw::cpcl::bitmap_label(
                        &mono,
                        render_dpi.clamp(72, u16::MAX as u32) as u16,
                        copies.max(1),
                    )
                    .map_err(|e| BackendError::fatal("CPCL_ENCODE_ERROR", &e.to_string()))
                } else {
                    Ok(print_raw::epl::epl_label_copies_with_gap(
                        &mono,
                        copies,
                        Self::label_gap_dots(label_gap_mm, render_dpi),
                    ))
                }
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
                        .map_err(Self::map_image_error)?;
                Self::repeat_raw_copies(&single, copies)
            }
            _ => Err(BackendError::fatal(
                "UNKNOWN_MODE",
                &format!("unsupported print mode {mode:?} for an image"),
            )),
        }
    }
}

/// The document GDI path (media validation + PDFium render),
/// shared by native GDI jobs and by the pre-write rescue for
/// raw-mode documents whose driver refused the RAW document
/// (GDI-only POS/label drivers - see the ("pdf", raw) arm).
fn print_pdf_via_gdi(
    target_name: &str,
    title: &str,
    data: &[u8],
    opts: &JobOptions,
) -> Result<(), BackendError> {
    let policy = NativePrintBackend::parse_scale_policy(opts.scale_policy.as_deref());
    // Validate page geometry against the current Windows driver.
    // Both failures here are RETRYABLE (2026-11-05
    // practicality round): a capabilities read fails
    // transiently while the spooler/driver is busy or
    // initializing, and an empty form list at printer
    // power-on is the same condition one read later.
    // A genuinely broken driver still fails — after the
    // retry envelope, with the same codes.
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
        paper: opts.paper.clone(),
    };
    print_win::print_pdf_gdi_detailed(target_name, title, data, &gdi_opts).map_err(|e| {
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
    })?;
    Ok(())
}

fn parse_paper_width_mm(paper: Option<&str>) -> Option<f64> {
    let s = paper?.trim().to_ascii_lowercase();
    let head = s.split(|c: char| !c.is_ascii_digit() && c != '.').next()?;
    let mm: f64 = head.parse().ok()?;
    // Restrict fallback parsing to narrow receipt widths so A4/letter names don't hijack roll sizing.
    if (30.0..=120.0).contains(&mm) && s.contains("mm") {
        Some(mm)
    } else {
        None
    }
}

fn print_png_via_gdi(
    target_name: &str,
    title: &str,
    data: &[u8],
    opts: &JobOptions,
) -> Result<(), BackendError> {
    let canvas = print_render::decode_png(data)
        .map_err(|e| BackendError::fatal("IMAGE_DECODE_ERROR", &e.to_string()))?;
    print_win::print_canvas_gdi_configured(
        target_name,
        title,
        &canvas,
        opts.copies.max(1),
        opts.tray.as_deref(),
        opts.media_roll_width_mm
            .or(opts.source_width_mm)
            .or_else(|| parse_paper_width_mm(opts.paper.as_deref())),
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

impl PrintBackend for &mut NativePrintBackend {
    fn print(&mut self, job: &Job) -> Result<(), BackendError> {
        <NativePrintBackend as PrintBackend>::print(*self, job)
    }
}

impl PrintBackend for NativePrintBackend {
    fn print(&mut self, job: &Job) -> Result<(), BackendError> {
        // Point 3 independent Ed25519 license verification before hardware dispatch
        // (prevents bypassing IPC ingress by patching a single gate; calibration test prints stay ungated).
        let is_calibration = job.id.starts_with("cal-")
            && job
                .report
                .as_deref()
                .is_some_and(|r| r.starts_with("calibration."));
        if !is_calibration {
            let origin = read_lock(&self.configured_origin)
                .clone()
                .unwrap_or_default();
            let key = read_lock(&self.license_key).clone();
            let now = print_core::license::now_utc_secs();
            let last = *read_lock(&self.last_seen_utc);
            if let Err(err) =
                print_core::license::verify_license(key.as_deref(), &origin, now, last)
            {
                return Err(BackendError::fatal(
                    err.code(),
                    "license verification failed",
                ));
            }
        }
        let (kind, data, mut opts) = self.load_job_data(&job.id)?;
        // Per-printer defaults from the Control Center fill what
        // this job did not specify (owner brief, 2026-11-05),
        // applied through the scoped helper: a saved DOCUMENT
        // paper must never land on a receipt job (field incident,
        // 2026-11-05: a sheet paper saved for a POS printer in the
        // Control Center reached receipt_width_dots as the paper
        // override and killed every receipt with
        // AUTO_MEDIA_UNAVAILABLE). Looked up by the job's target
        // pattern here, and again by the resolved spooler name in
        // the spooler branch below.
        let settings_map = read_lock(&self.printer_settings).clone();
        if let Some(defaults) = settings_map.get(&job.printer) {
            Self::apply_printer_defaults(&mut opts, &kind, defaults);
        }
        let mut paper: Option<String> = opts.paper.clone();
        let title = job.report.as_deref().unwrap_or("Itbaa Job");

        let trimmed_printer = job.printer.trim();
        let lower_printer = trimmed_printer.to_ascii_lowercase();

        // 1. Direct TCP 9100 transport (tcp://host:port or raw://host:port)
        if lower_printer.starts_with("tcp://") || lower_printer.starts_with("raw://") {
            let (host, port, _) = Self::parse_host_port(trimmed_printer, 9100)?;
            Self::reject_metadata_resolution(&host, port)?;
            // Live sensor check for ESC/POS printers (2026-11-07):
            // ask the hardware itself (DLE EOT) before sending —
            // paper-end, a cutter fault or an offline/cover-open
            // state fail here with the sensor's own code instead of
            // vanishing into the printer. A printer that does not
            // answer the status protocol is NOT blocked: the probe
            // may veto only on positive evidence, never on silence.
            if opts.mode == "escpos" {
                // Temporarily skip TCP hosts that ignore DLE EOT (or were transiently unreachable)
                // so we don't stall 700ms on every receipt, while expiring entries after DLE_SILENT_TTL
                // so recovered printers regain live hardware health checks.
                static DLE_SILENT: std::sync::OnceLock<Mutex<HashMap<String, Instant>>> =
                    std::sync::OnceLock::new();
                let silent_map = DLE_SILENT.get_or_init(|| Mutex::new(HashMap::new()));
                let addr_key = format!("{host}:{port}");
                let skip_probe = {
                    let mut map = silent_map.lock().unwrap_or_else(|e| e.into_inner());
                    dle_silent_is_active(&mut map, &addr_key, Instant::now())
                };
                if !skip_probe {
                    match print_raw::tcp9100::query_blocking_condition(
                        &host,
                        port,
                        DLE_PROBE_TIMEOUT,
                    ) {
                        Ok(Some((code, message))) => {
                            return Err(BackendError::retryable(code, message));
                        }
                        Ok(None) => {}
                        Err(_) => {
                            let mut map = silent_map.lock().unwrap_or_else(|e| e.into_inner());
                            dle_silent_record(&mut map, addr_key, Instant::now());
                        }
                    }
                }
            }
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
                        paper: paper.as_deref(),
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
                        paper: paper.as_deref(),
                        target_width_dots: media_width_dots,
                        copies: opts.copies,
                        label_box: None,
                        render_dpi: 203,
                        label_gap_mm: opts.label_gap_mm,
                    },
                )?,
                _ => Self::repeat_raw_copies(&data, opts.copies)?,
            };
            // A connect-phase failure proves zero bytes reached
            // the printer (it is off, asleep, or unreachable), so
            // the job retries safely; only a write-phase failure is
            // ambiguous (2026-11-05 practicality round).
            print_raw::tcp9100::send_raw_phased(
                &host,
                port,
                &raw_bytes,
                TCP_CONNECT_TIMEOUT,
                TCP_WRITE_TIMEOUT,
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
        if lower_printer.starts_with("ipp://") || opts.mode == "ipp" {
            if !lower_printer.starts_with("ipp://") && !lower_printer.starts_with("http://") {
                return Err(BackendError::fatal(
                    "BAD_PRINTER_ADDRESS",
                    "IPP mode requires an ipp:// or http:// printer URI",
                ));
            }
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
            let (host, port, path) = Self::parse_host_port(trimmed_printer, 631)?;
            Self::reject_metadata_resolution(&host, port)?;
            let mut client = print_ipp::IppClient::new(&host, port);
            client.path = path.clone();
            client.printer_uri = Self::ipp_uri(&host, port, &path);
            let caps = client.get_printer_attributes().map_err(|e| match e {
                print_ipp::IppError::Http(code) if (400..500).contains(&code) => {
                    BackendError::fatal("IPP_CLIENT_ERROR", &e.to_string())
                }
                other => BackendError::retryable("IPP_ATTRIBUTES_ERROR", &other.to_string()),
            })?;
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
                match print_ipp::tray_to_media_source(tray_name) {
                    Some(source) => ipp_options.media_source = Some(source),
                    None => {
                        eprintln!(
                            "Itbaa: IPP tray {tray_name:?} unmapped; using printer default tray"
                        );
                    }
                }
            }
            if opts.duplex == Some(true) {
                let sides = "two-sided-long-edge";
                if caps.sides_supported.is_empty()
                    || caps.sides_supported.iter().any(|x| x == sides)
                {
                    ipp_options.sides = Some(sides.to_string());
                } else {
                    eprintln!("Itbaa: IPP printer does not report duplex; printing single-sided");
                }
            }
            if let Some(paper_name) = opts.paper.as_deref() {
                let lower = paper_name.trim().to_ascii_lowercase();
                let mapped = match lower.as_str() {
                    "a4" => Some("iso_a4_210x297mm"),
                    "a5" => Some("iso_a5_148x210mm"),
                    "letter" => Some("na_letter_8.5x11in"),
                    "legal" => Some("na_legal_8.5x14in"),
                    _ => None,
                };
                let matched_media = if caps.media_supported.is_empty() {
                    mapped.map(|s| s.to_string())
                } else if let Some(m) = mapped {
                    caps.media_supported
                        .iter()
                        .find(|x| {
                            x.eq_ignore_ascii_case(m)
                                || x.to_ascii_lowercase().contains(&format!("_{lower}_"))
                        })
                        .cloned()
                } else {
                    caps.media_supported
                        .iter()
                        .find(|x| x.eq_ignore_ascii_case(paper_name.trim()))
                        .cloned()
                };
                match matched_media {
                    Some(m) => ipp_options.media = Some(m),
                    None => {
                        eprintln!(
                            "Itbaa: IPP paper {paper_name:?} unmapped; using printer default media"
                        );
                    }
                }
            }
            ipp_options.color_mode = Some(
                if opts.color && caps.color_supported {
                    "color"
                } else {
                    "monochrome"
                }
                .to_string(),
            );
            client
                .print_job_with_options(&data, fmt, title, opts.copies.max(1), &ipp_options)
                .map_err(|e| BackendError::ambiguous("IPP_PRINT_ERROR", &e.to_string()))?;
            return Ok(());
        }

        // 3. Windows Spooler / Local / Shared / Virtual printer
        let mut discovered = print_win::discover_printers().map_err(|e| {
            BackendError::retryable("SPOOLER_UNAVAILABLE", &format!("discover_printers: {e}"))
        })?;
        if resolve_printer(&job.printer, &discovered).is_none() {
            // Re-probe once with a fresh cache in case a USB/shared printer just reconnected.
            print_win::invalidate_discovery_cache();
            if let Ok(fresh) = print_win::discover_printers() {
                discovered = fresh;
            }
        }
        let mut target_kind: Option<PrinterKind> = None;
        let mut target_class: Option<String> = None;
        let target_name = if let Some(matched) = resolve_printer(&job.printer, &discovered) {
            target_kind = Some(matched.kind);
            target_class = Some(matched.printer_class.clone());
            matched.name.clone()
        } else if !discovered.is_empty() {
            return Err(BackendError::retryable(
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
        // The device class drives the raw-language target guard
        // (office printers render GDI only). From the matched
        // queue when there is one; otherwise detected from the
        // queue name, the same evidence the page had.
        let target_class = target_class.unwrap_or_else(|| {
            let kind = target_kind.unwrap_or(PrinterKind::Local);
            print_core::printer::detect_protocol(&target_name, "", kind)
                .1
                .to_string()
        });

        // Per-printer defaults, second lookup by the RESOLVED
        // spooler name (the first lookup used the job's target
        // pattern; a pattern resolves to a concrete queue whose
        // Control Center settings live under its real name).
        // Fill-if-unspecified semantics make this a no-op when the
        // first lookup already applied the same entry.
        if target_name != job.printer {
            if let Some(defaults) = settings_map.get(&target_name) {
                Self::apply_printer_defaults(&mut opts, &kind, defaults);
                paper = opts.paper.clone();
            }
        }

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
                print_core::printer::document_mode_downgrade(&kind, &opts.mode, pk, &target_class)
            {
                opts.mode = downgraded.to_string();
            }
        }

        // Raw printer language can never go to a virtual PDF/XPS
        // writer (the spooler would write the raw bytes into a
        // document file, producing a corrupt, unopenable "PDF") nor
        // to an office printer (a GDI-only queue can only print the
        // bytes as garbage). Documents never reach this check in a
        // raw mode: the downgrade above re-routed them to GDI.
        if let Some(pk) = target_kind {
            if let Some(code) =
                print_core::printer::raw_target_block(&kind, &opts.mode, pk, &target_class)
            {
                let message = if code == "RAW_TO_OFFICE_PRINTER" {
                    "Raw printer language (ESC/POS, ZPL, TSPL, EPL) is not supported on office printers. Select a compatible thermal or label printer."
                } else {
                    "Raw printer language (ESC/POS, ZPL, TSPL, EPL) cannot print to a virtual PDF/XPS printer. Select a physical printer."
                };
                return Err(BackendError::fatal(code, message));
            }
        }

        // Do not pre-block on Windows Spooler health flags: sleeping USB/WSD
        // printers often report OFFLINE/PAUSED/USER_INTERVENTION until a job
        // is submitted to the spooler. Let the spooler accept and wake the device.

        let target_width_dots =
            if opts.mode == "escpos" && matches!(kind.as_str(), "pdf" | "image-png") {
                Some(Self::receipt_width_dots(
                    &target_name,
                    paper.as_deref(),
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
        let (label_box, render_dpi) =
            if matches!(opts.mode.as_str(), "zpl" | "tspl" | "epl" | "cpcl")
                && matches!(kind.as_str(), "pdf" | "image-png")
            {
                match print_win::printer_capabilities(&target_name) {
                    Ok(caps) => resolve_label_box_and_dpi(&caps),
                    Err(_) => (None, 203),
                }
            } else {
                (None, 203)
            };

        match (kind.as_str(), opts.mode.as_str()) {
            ("pdf", "escpos" | "zpl" | "tspl" | "epl" | "cpcl") => {
                let raw_bytes = Self::pdf_to_raw_stream(
                    &data,
                    RawRender {
                        mode: &opts.mode,
                        paper: paper.as_deref(),
                        target_width_dots,
                        copies: opts.copies,
                        label_box,
                        render_dpi,
                        label_gap_mm: opts.label_gap_mm,
                    },
                    opts.cut,
                )?;
                match print_win::print_raw_phased(&target_name, title, &raw_bytes) {
                    Ok(_) => Ok(()),
                    Err(e) if e.pre_write => {
                        // The spooler/driver refused the RAW
                        // document before a single payload byte was
                        // accepted — the signature of a GDI-only
                        // POS/label driver (2026-11-06 field round:
                        // real POS printers died here as an
                        // ambiguous dead end). Nothing was printed,
                        // so rendering the same document through
                        // GDI cannot duplicate it (ADR-0004-safe).
                        eprintln!(
                            "Itbaa: raw send to {target_name:?} refused pre-write ({}); falling back to GDI rendering",
                            e.message
                        );
                        print_pdf_via_gdi(&target_name, title, &data, &opts)
                    }
                    Err(e) => Err(BackendError::ambiguous("SPOOLER_RAW_ERROR", &e.message)),
                }
            }
            ("pdf", "gdi") => print_pdf_via_gdi(&target_name, title, &data, &opts),
            ("pdf", _) => Err(BackendError::fatal(
                "UNKNOWN_MODE",
                &format!("unsupported print mode {:?} for this document", opts.mode),
            )),
            ("image-png", "gdi") => print_png_via_gdi(&target_name, title, &data, &opts),
            ("image-png", "escpos" | "zpl" | "tspl" | "epl" | "cpcl") => {
                let raw_bytes = Self::png_to_raw_stream(
                    &data,
                    RawRender {
                        mode: &opts.mode,
                        paper: paper.as_deref(),
                        target_width_dots,
                        copies: opts.copies,
                        label_box,
                        render_dpi,
                        label_gap_mm: opts.label_gap_mm,
                    },
                )?;
                match print_win::print_raw_phased(&target_name, title, &raw_bytes) {
                    Ok(_) => Ok(()),
                    Err(e) if e.pre_write => {
                        // Same GDI-only-driver rescue as the PDF
                        // arm: the RAW document was refused before
                        // any byte was accepted, so printing the
                        // receipt image through GDI (every Windows
                        // driver renders it, sized to the roll the
                        // job reported) cannot duplicate anything.
                        // This is the difference between a POS
                        // receipt that prints on any printer and
                        // one that dies as "wrong printer".
                        eprintln!(
                            "Itbaa: raw send to {target_name:?} refused pre-write ({}); printing the image through GDI instead",
                            e.message
                        );
                        print_png_via_gdi(&target_name, title, &data, &opts)
                    }
                    Err(e) => Err(BackendError::ambiguous("SPOOLER_RAW_ERROR", &e.message)),
                }
            }
            ("image-png", _) => Err(BackendError::fatal(
                "UNKNOWN_MODE",
                &format!("unsupported print mode {:?} for this image", opts.mode),
            )),
            _ => {
                // Pre-rendered raw payloads have no GDI rendering
                // to rescue into, but their failure classification
                // still follows the phase: a pre-write refusal
                // printed nothing and may retry; a write failure
                // stays ambiguous (ADR-0004).
                for copy_idx in 0..opts.copies.max(1) {
                    print_win::print_raw_phased(&target_name, title, &data).map_err(|e| {
                        if e.pre_write && copy_idx == 0 {
                            BackendError::retryable("SPOOLER_RAW_ERROR", &e.message)
                        } else {
                            BackendError::ambiguous("SPOOLER_RAW_ERROR", &e.message)
                        }
                    })?;
                }
                Ok(())
            }
        }
    }
}

fn outcome_log_fields(outcome: &Outcome) -> Option<(&'static str, &str)> {
    match outcome {
        Outcome::Done(id) => Some(("done", id.as_str())),
        Outcome::Retried(id) => Some(("retried", id.as_str())),
        Outcome::Failed(id) => Some(("failed", id.as_str())),
        Outcome::DeadLetter(id) => Some(("dead_letter", id.as_str())),
        Outcome::Idle | Outcome::Skipped => None,
    }
}

/// Spawn the background worker thread that drains queued jobs across all
/// printer patterns using `print_core::Worker`.
pub fn spawn_worker_loop(
    db_path: String,
    printer_settings: Arc<RwLock<config::PrinterSettingsMap>>,
    configured_origin: Arc<RwLock<Option<String>>>,
    license_key: Arc<RwLock<Option<String>>>,
    last_seen_utc: Arc<RwLock<u64>>,
    wake: Arc<(Mutex<bool>, Condvar)>,
    stop: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        if let Ok(mut retention_queue) = Queue::open(&db_path) {
            if let Err(e) = retention_queue.purge_stale_failed_payloads(30 * 86_400) {
                eprintln!("Itbaa: failed-payload retention purge failed: {e}");
            }
            if let Err(e) = retention_queue.purge_stale_failed_jobs(90 * 86_400) {
                eprintln!("Itbaa: failed-job row retention purge failed: {e}");
            }
        }
        let mut entropy: u64 = 0x5EED_2026;
        let mut coord_queue: Option<Queue> = None;
        let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let backoff: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
        let mut last_reconcile: Option<Instant> = None;
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let mut dispatched = false;
            if coord_queue.is_none() {
                match Queue::open(&db_path) {
                    Ok(q) => coord_queue = Some(q),
                    Err(e) => eprintln!("Itbaa: cannot open the print queue: {e}"),
                }
            }
            let mut reset_queue = false;
            if let Some(queue) = coord_queue.as_mut() {
                if last_reconcile.map_or(true, |t| t.elapsed() >= Duration::from_secs(15)) {
                    last_reconcile = Some(Instant::now());
                    if let Err(e) = queue.reconcile_stale() {
                        eprintln!("Itbaa: stale-job reconciliation failed: {e}");
                        reset_queue = true;
                    }
                }
                let printers = if reset_queue {
                    Vec::new()
                } else {
                    match queue.queued_printers() {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("Itbaa: queued_printers failed: {e}");
                            reset_queue = true;
                            Vec::new()
                        }
                    }
                };
                for printer in printers {
                    let now = Instant::now();
                    {
                        let mut cd = backoff.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(&until) = cd.get(&printer) {
                            if now < until {
                                continue;
                            }
                            cd.remove(&printer);
                        }
                        if cd.len() > 64 {
                            cd.retain(|_, until| now < *until);
                        }
                    }
                    let claimed_slot = {
                        let mut active = in_flight.lock().unwrap_or_else(|e| e.into_inner());
                        if active.len() >= MAX_CONCURRENT_PRINTERS {
                            break;
                        }
                        active.insert(printer.clone())
                    };
                    if !claimed_slot {
                        continue;
                    }
                    dispatched = true;
                    entropy = entropy.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let seed = entropy;
                    let db_c = db_path.clone();
                    let ps_c = Arc::clone(&printer_settings);
                    let origin_c = Arc::clone(&configured_origin);
                    let lic_c = Arc::clone(&license_key);
                    let last_c = Arc::clone(&last_seen_utc);
                    let wake_c = Arc::clone(&wake);
                    let stop_c = Arc::clone(&stop);
                    let flight_c = Arc::clone(&in_flight);
                    let backoff_c = Arc::clone(&backoff);
                    // One short-lived thread per active printer prevents a slow network
                    // printer from blocking local USB receipt printers.
                    std::thread::spawn(move || {
                        struct SlotGuard {
                            in_flight: Arc<Mutex<HashSet<String>>>,
                            backoff: Arc<Mutex<HashMap<String, Instant>>>,
                            printer: String,
                            wake: Arc<(Mutex<bool>, Condvar)>,
                            normal_exit: bool,
                        }
                        impl Drop for SlotGuard {
                            fn drop(&mut self) {
                                self.in_flight
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .remove(&self.printer);
                                if self.normal_exit {
                                    let (lock, cvar) = &*self.wake;
                                    let mut pending =
                                        lock.lock().unwrap_or_else(|p| p.into_inner());
                                    *pending = true;
                                    cvar.notify_one();
                                } else {
                                    self.backoff
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .insert(
                                            self.printer.clone(),
                                            Instant::now() + WORKER_ERROR_BACKOFF,
                                        );
                                }
                            }
                        }
                        let mut guard = SlotGuard {
                            in_flight: flight_c,
                            backoff: backoff_c,
                            printer: printer.clone(),
                            wake: wake_c,
                            normal_exit: false,
                        };
                        let Ok(mut q) = Queue::open(&db_c) else {
                            return;
                        };
                        let mut backend = NativePrintBackend {
                            db_path: db_c,
                            printer_settings: ps_c,
                            configured_origin: origin_c,
                            license_key: lic_c,
                            last_seen_utc: last_c,
                            read_queue: None,
                        };
                        let mut local_entropy = seed;
                        while !stop_c.load(Ordering::Acquire) {
                            local_entropy = local_entropy
                                .wrapping_mul(6364136223846793005)
                                .wrapping_add(1);
                            let mut worker = Worker {
                                queue: &mut q,
                                backend: &mut backend,
                                cfg: WorkerConfig::default(),
                                entropy: local_entropy,
                            };
                            let started = Instant::now();
                            let outcome =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    worker.run_once(&printer)
                                }));
                            let elapsed_ms = started.elapsed().as_millis();
                            drop(worker);
                            match outcome {
                                Ok(Ok(o)) => {
                                    if let Some((label, job_id)) = outcome_log_fields(&o) {
                                        eprintln!(
                                            "Itbaa: job_id={job_id} printer={printer:?} outcome={label} elapsed_ms={elapsed_ms}"
                                        );
                                    }
                                    if o == Outcome::Idle {
                                        guard.normal_exit = true;
                                        break;
                                    }
                                }
                                Ok(Err(e)) => {
                                    eprintln!(
                                        "Itbaa: worker error printer={printer:?} elapsed_ms={elapsed_ms}: {e}"
                                    );
                                    break;
                                }
                                Err(_) => {
                                    eprintln!(
                                        "Itbaa: worker panicked printer={printer:?} elapsed_ms={elapsed_ms}"
                                    );
                                    break;
                                }
                            }
                        }
                    });
                }
            }
            if reset_queue {
                coord_queue = None;
            }
            if !dispatched {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let (lock, cvar) = &*wake;
                let mut pending = lock.lock().unwrap_or_else(|p| p.into_inner());
                if !*pending && !stop.load(Ordering::Acquire) {
                    let (mut guard, _) = cvar
                        .wait_timeout(pending, Duration::from_secs(2))
                        .unwrap_or_else(|p| p.into_inner());
                    *guard = false;
                } else {
                    *pending = false;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn printer_defaults_never_poison_receipts() {
        // The 2026-11-05 field incident: a sheet paper saved for
        // a POS printer in the Control Center must not reach a
        // receipt job (it killed receipts as an unknown paper
        // override), while document jobs still inherit it.
        let defaults = config::PrinterSettings {
            paper: Some("A4".to_string()),
            tray: Some("Tray 2".to_string()),
            duplex: Some(true),
            color: Some(false),
        };
        let mut receipt = JobOptions {
            mode: "escpos".to_string(),
            ..Default::default()
        };
        NativePrintBackend::apply_printer_defaults(&mut receipt, "image-png", &defaults);
        assert!(receipt.paper.is_none(), "receipt paper must stay unset");
        assert!(
            receipt.duplex.is_none() && receipt.color,
            "receipt flags untouched"
        );
        assert_eq!(receipt.tray.as_deref(), Some("Tray 2"));

        let mut doc = JobOptions {
            mode: "gdi".to_string(),
            ..Default::default()
        };
        NativePrintBackend::apply_printer_defaults(&mut doc, "pdf", &defaults);
        assert_eq!(doc.paper.as_deref(), Some("A4"));
        assert!(doc.duplex == Some(true) && !doc.color);

        // A document carrying its own paper keeps it.
        let mut own = JobOptions {
            mode: "gdi".to_string(),
            paper: Some("A5".to_string()),
            ..Default::default()
        };
        NativePrintBackend::apply_printer_defaults(&mut own, "pdf", &defaults);
        assert_eq!(own.paper.as_deref(), Some("A5"));
    }

    #[test]
    fn auto_document_form_is_derived_from_caps() {
        let mk = |name: &str, w: f64, h: f64, roll: bool| print_core::Form {
            name: name.to_string(),
            size: print_core::PageSize::new(w, h),
            roll,
            paper_id: None,
        };
        let caps = print_core::PrinterCapabilities {
            forms: vec![mk("A5", 148.0, 210.0, false), mk("A4", 210.0, 297.0, false)],
            ..Default::default()
        };
        assert_eq!(auto_document_form(&caps).unwrap().0, "A4");
        let caps = print_core::PrinterCapabilities {
            forms: vec![
                mk("Legal", 215.9, 355.6, false),
                mk("A5", 148.0, 210.0, false),
            ],
            ..Default::default()
        };
        assert_eq!(auto_document_form(&caps).unwrap().0, "Legal");
        let caps = print_core::PrinterCapabilities {
            forms: vec![mk("80mm Roll", 80.0, 3276.0, true)],
            ..Default::default()
        };
        let f = auto_document_form(&caps).unwrap();
        assert_eq!((f.0.as_str(), f.1, f.2), ("80mm Roll", 80.0, 297.0));
        assert_eq!(
            find_caps_form(&caps, "80MM ROLL").map(|f| f.0),
            Some("80mm Roll".to_string())
        );
    }

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
        // 300-dpi head aligns to whole bytes; 600-dpi Generic/Text pseudo-res stays 203 dpi.
        let caps300 = print_core::PrinterCapabilities {
            resolutions_dpi: vec![300],
            ..Default::default()
        };
        let scaled300 = NativePrintBackend::scale_head_dots(576, &caps300);
        assert_eq!(scaled300 % 8, 0);
        assert_eq!(scaled300, 848);
        let caps600 = print_core::PrinterCapabilities {
            resolutions_dpi: vec![600],
            ..Default::default()
        };
        assert_eq!(NativePrintBackend::scale_head_dots(576, &caps600), 576);
        assert!(NativePrintBackend::is_office_sheet_or_envelope(
            "Envelope #9"
        ));
        assert!(!NativePrintBackend::is_office_sheet_or_envelope(
            "80mm Roll"
        ));
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
        assert_eq!(tab_title_for("http://tauri.localhost/"), "Control Center");
        assert_eq!(
            tab_title_for("http://tauri.localhost/settings.html"),
            "Settings"
        );
        assert_eq!(tab_title_for("tauri://localhost/boot.html"), "Itbaa");
        assert!(is_local_app_url(
            &tauri::Url::parse("tauri://localhost/index.html").unwrap()
        ));
        assert!(is_local_app_url(
            &tauri::Url::parse("http://tauri.localhost/index.html").unwrap()
        ));
        assert!(is_local_app_url(
            &tauri::Url::parse("https://tauri.localhost/index.html").unwrap()
        ));
        assert!(!is_local_app_url(
            &tauri::Url::parse("http://tauri.localhost:8080/index.html").unwrap()
        ));
        assert!(!is_local_app_url(
            &tauri::Url::parse("tauri://localhost:1234/index.html").unwrap()
        ));
        assert!(!is_local_app_url(
            &tauri::Url::parse("http://user@tauri.localhost/index.html").unwrap()
        ));
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
        tabs.set_url("tab-closed", "http://h/pos/ui");
        assert!(tabs.url_of("tab-closed").is_none());
        // POS tab keeps is_pos=true across login redirects.
        tabs.set_url("tab-2", "http://h/web/login?redirect=/pos/ui");
        let after_redirect = tabs.snapshot();
        assert!(after_redirect[1].is_pos);
        assert_eq!(after_redirect[1].title, "Sign in");
    }

    #[test]
    fn base64_and_host_port_parsing() {
        let decoded = decode_base64("JVBERi0xLjQ=").unwrap();
        assert_eq!(decoded, b"%PDF-1.4");
        assert!(decode_base64("A").is_err());
        assert!(decode_base64("A===").is_err());
        assert!(decode_base64("JV==AAAA").is_err());
        assert!(decode_base64("AB=C").is_err());
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
        let (hn, pn, _) =
            NativePrintBackend::parse_host_port("ipp://printer.local:631/ipp/print", 631).unwrap();
        assert_eq!((hn.as_str(), pn), ("printer.local", 631));
        for bad in [
            "tcp://192.168.1.200:0",
            "ipp://[::1]:0/ipp/print",
            "tcp://169.254.169.254:80",
            "ipp://169.254.169.254:631/ipp/print",
            "tcp://[::ffff:169.254.169.254]:9100",
            "tcp://[::169.254.169.254]:9100",
            "tcp://[64:ff9b::169.254.169.254]:9100",
            "tcp://[fd00:ec2::254]:80",
            "tcp://169.254.170.2:80",
            "tcp://100.100.100.200:80",
            "ipp://metadata.google.internal:80/computeMetadata/v1/",
            "ipp://192.168.1.50:631/ipp/print\r\nX-Injected: 1",
            "tcp://   :9100",
            "tcp://user@192.168.1.10:9100",
            "tcp://bad host:9100",
            "tcp://999.999.999.999:9100",
            "tcp://2852039166:80",
            "ipp://[not-ipv6]:631/ipp/print",
        ] {
            let err = NativePrintBackend::parse_host_port(bad, 9100).unwrap_err();
            assert_eq!(err.code, "BAD_PRINTER_ADDRESS", "expected reject for {bad}");
        }
        assert!(NativePrintBackend::parse_host_port("tcp://192.168.1.200", 0).is_err());
    }

    #[test]
    fn repeat_raw_copies_enforces_cap() {
        assert_eq!(
            NativePrintBackend::repeat_raw_copies(b"abc", 3).unwrap(),
            b"abcabcabc"
        );
        assert_eq!(
            NativePrintBackend::repeat_raw_copies(b"abc", 0).unwrap(),
            b"abc"
        );
        let one_mib = vec![0u8; 1024 * 1024];
        let err = NativePrintBackend::repeat_raw_copies(&one_mib, 101).unwrap_err();
        assert_eq!(err.code, "PAYLOAD_TOO_LARGE");
        assert!(!err.retryable);
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
        assert_eq!(NativePrintBackend::label_gap_mm(Some(f64::NAN)), 3.0);
        assert_eq!(NativePrintBackend::label_gap_mm(Some(f64::INFINITY)), 3.0);
        assert_eq!(NativePrintBackend::label_gap_mm(Some(500.0)), 100.0);
        // 3 mm at 203 dpi is the EPL default of 24 dots.
        assert_eq!(NativePrintBackend::label_gap_dots(None, 203), 24);
        assert_eq!(NativePrintBackend::label_gap_dots(Some(3.0), 300), 35);
    }

    #[test]
    fn backend_reuses_read_queue_without_reopening() {
        let mut q = Queue::in_memory().unwrap();
        let job = Job {
            id: "job-reuse-1".to_string(),
            idempotency_key: "job-reuse-1".to_string(),
            printer: "p1".to_string(),
            report: None,
            pages: 1,
            bytes: 8,
            priority: 0,
            state: JobState::Queued,
            attempts: 0,
            error_code: None,
        };
        q.enqueue(&job).unwrap();
        q.store_payload("job-reuse-1", "pdf", b"%PDF-1.4").unwrap();
        let mut backend = NativePrintBackend {
            db_path: String::new(),
            printer_settings: Arc::new(RwLock::new(config::PrinterSettingsMap::new())),
            configured_origin: Arc::new(RwLock::new(None)),
            license_key: Arc::new(RwLock::new(None)),
            last_seen_utc: Arc::new(RwLock::new(0)),
            read_queue: Some(q),
        };
        let (kind1, data1, _) = backend.load_job_data("job-reuse-1").unwrap();
        let (kind2, data2, _) = backend.load_job_data("job-reuse-1").unwrap();
        assert_eq!(kind1, "pdf");
        assert_eq!(data1, b"%PDF-1.4");
        assert_eq!(kind2, "pdf");
        assert_eq!(data2, b"%PDF-1.4");
        let missing = backend.load_job_data("missing-job").unwrap_err();
        assert_eq!(missing.code, "MISSING_PAYLOAD");
        assert!(!missing.retryable);
        assert!(backend.read_queue.is_some());
    }

    #[test]
    fn outcome_log_fields_maps_terminal_and_retry_outcomes() {
        assert_eq!(outcome_log_fields(&Outcome::Idle), None);
        assert_eq!(outcome_log_fields(&Outcome::Skipped), None);
        assert_eq!(
            outcome_log_fields(&Outcome::Done("j1".into())),
            Some(("done", "j1"))
        );
        assert_eq!(
            outcome_log_fields(&Outcome::Retried("j2".into())),
            Some(("retried", "j2"))
        );
        assert_eq!(
            outcome_log_fields(&Outcome::Failed("j3".into())),
            Some(("failed", "j3"))
        );
        assert_eq!(
            outcome_log_fields(&Outcome::DeadLetter("j4".into())),
            Some(("dead_letter", "j4"))
        );
    }

    #[test]
    fn diagnostics_license_metadata_omits_secrets_and_pii() {
        let now = 1_700_000_000u64;
        let timed = print_core::license::LicenseStatus::from_result(&Ok(
            print_core::license::LicensePayload {
                c: "Acme Secret Co".to_string(),
                o: "https://erp.secret.local".to_string(),
                exp: now + 10 * 86_400 + 3_600,
                t: "pro".to_string(),
            },
        ));
        let mut val = serde_json::json!({});
        insert_license_diagnostics(&mut val, &timed, now);
        assert_eq!(val["license_valid"], true);
        assert_eq!(val["license_code"], "OK");
        assert_eq!(val["license_tier"], "pro");
        assert_eq!(val["license_perpetual"], false);
        assert_eq!(val["license_days_remaining"], 10);
        let s = val.to_string();
        assert!(!s.contains("Acme Secret Co"));
        assert!(!s.contains("erp.secret.local"));
        assert!(!s.contains("ITBAA1."));

        let perpetual = print_core::license::LicenseStatus::from_result(&Ok(
            print_core::license::LicensePayload {
                c: "Acme Secret Co".to_string(),
                o: "https://erp.secret.local".to_string(),
                exp: 0,
                t: "pro".to_string(),
            },
        ));
        let mut pval = serde_json::json!({});
        insert_license_diagnostics(&mut pval, &perpetual, now);
        assert_eq!(pval["license_valid"], true);
        assert_eq!(pval["license_perpetual"], true);
        assert!(pval["license_days_remaining"].is_null());

        let expired = print_core::license::LicenseStatus::from_result(&Err(
            print_core::license::LicenseError::Expired,
        ));
        let mut eval = serde_json::json!({});
        insert_license_diagnostics(&mut eval, &expired, now);
        assert_eq!(eval["license_valid"], false);
        assert_eq!(eval["license_code"], "ERR_LICENSE_EXPIRED");
        assert!(eval["license_tier"].is_null());
        assert_eq!(eval["license_perpetual"], false);
        assert!(eval["license_days_remaining"].is_null());
    }

    #[test]
    fn diagnostics_environment_redacts_origin_and_tokens() {
        let raw = serde_json::json!({
            "href": "https://erp.secret.local/odoo?access_token=tok123#frag",
            "pathname": "/odoo?access_token=tok123",
            "userAgent": "Edg/130.0",
            "secureContext": true,
            "es2022": true,
            "injectedKey": "should-be-dropped",
            "errors": [
                "load: https://user:pw@erp.secret.local:8069/web/assets/bundle.js?token=abc failed to load",
                "error: plain TypeError"
            ]
        });
        let redacted = redact_environment_for_diagnostics(&raw);
        let serialized = redacted.to_string();
        assert!(!serialized.contains("erp.secret.local"));
        assert!(!serialized.contains("tok123"));
        assert!(!serialized.contains("token=abc"));
        assert!(!serialized.contains("user:pw"));
        assert!(!serialized.contains("injectedKey"));
        assert_eq!(redacted["pathname"], "/odoo");
        assert_eq!(redacted["secureContext"], true);
        assert_eq!(
            redacted["errors"][0],
            "load: [redacted-origin]/web/assets/bundle.js failed to load"
        );
        assert_eq!(redacted["errors"][1], "error: plain TypeError");
    }

    #[test]
    fn dle_silent_cache_expires_and_bounds_entries() {
        let mut map = HashMap::new();
        let t0 = Instant::now();
        assert!(!dle_silent_is_active(&mut map, "192.168.1.50:9100", t0));

        dle_silent_record(&mut map, "192.168.1.50:9100".to_string(), t0);
        assert!(dle_silent_is_active(
            &mut map,
            "192.168.1.50:9100",
            t0 + Duration::from_secs(60)
        ));
        // Expires at TTL and removes stale entry so live probing resumes.
        assert!(!dle_silent_is_active(
            &mut map,
            "192.168.1.50:9100",
            t0 + DLE_SILENT_TTL
        ));
        assert!(!map.contains_key("192.168.1.50:9100"));

        // Capacity stays bounded at DLE_SILENT_MAX_ENTRIES, evicting oldest when full.
        for i in 0..DLE_SILENT_MAX_ENTRIES {
            dle_silent_record(
                &mut map,
                format!("10.0.0.{i}:9100"),
                t0 + Duration::from_secs(i as u64),
            );
        }
        assert_eq!(map.len(), DLE_SILENT_MAX_ENTRIES);
        dle_silent_record(
            &mut map,
            "10.0.1.1:9100".to_string(),
            t0 + Duration::from_secs(DLE_SILENT_MAX_ENTRIES as u64),
        );
        assert_eq!(map.len(), DLE_SILENT_MAX_ENTRIES);
        assert!(map.contains_key("10.0.1.1:9100"));
        assert!(!map.contains_key("10.0.0.0:9100"));
    }

    #[test]
    fn remote_printer_pattern_ssrf_and_dns_fail_closed() {
        let empty = config::PrinterSettingsMap::new();
        // Remote ad-hoc network URIs: valid printer ports on LAN hosts pass.
        for ok in [
            "EPSON TM-T88V",
            "tcp://192.168.1.50:9100",
            "tcp://192.168.1.50",
            "raw://192.168.1.50:9109",
            "ipp://192.168.1.50:631/ipp/print",
            "http://192.168.1.50/ipp/print",
            "https://192.168.1.50:631/ipp/print",
        ] {
            assert!(
                validate_printer_pattern(ok, false, &empty).is_ok(),
                "expected remote allow for {ok}"
            );
        }
        // Remote ad-hoc network URIs: loopback hosts and non-printer ports are rejected.
        for bad in [
            "tcp://127.0.0.1:9100",
            "tcp://127.0.0.2:9100",
            "tcp://[::1]:9100",
            "tcp://[::ffff:127.0.0.1]:9100",
            "tcp://localhost:9100",
            "ipp://127.0.0.1:631/ipp/print",
            "ipp://[::1]:631/ipp/print",
            "http://localhost:631/ipp/print",
            "tcp://192.168.1.50:80",
            "raw://192.168.1.50:9110",
            "ipp://192.168.1.50:8080/ipp/print",
            "http://192.168.1.50:8069/web",
            "https://192.168.1.50:443/ipp/print",
            "TCP://127.0.0.1:9100",
        ] {
            assert!(
                validate_printer_pattern(bad, false, &empty).is_err(),
                "expected remote reject for {bad}"
            );
        }
        // Local pages and explicitly configured URIs in printer_settings remain allowed.
        assert!(validate_printer_pattern("ipp://[::1]:631/ipp/print", true, &empty).is_ok());
        let mut configured = config::PrinterSettingsMap::new();
        configured.insert(
            "tcp://127.0.0.1:9100".to_string(),
            config::PrinterSettings::default(),
        );
        assert!(validate_printer_pattern("tcp://127.0.0.1:9100", false, &configured).is_ok());
        let discovered = vec![PrinterInfo {
            name: "EPSON TM-T88".to_string(),
            driver: "Generic / Text Only".to_string(),
            port: "127.0.0.1".to_string(),
            kind: PrinterKind::Network,
            status: print_core::PrinterStatus::Ready,
            is_default: false,
            protocol: "escpos".to_string(),
            printer_class: "thermal".to_string(),
            stable_id: String::new(),
            health: None,
        }];
        assert!(is_discovered_loopback_printer(
            "tcp://127.0.0.1:9100",
            &discovered
        ));
        assert!(!is_discovered_loopback_printer(
            "tcp://127.0.0.1:9102",
            &discovered
        ));
        assert!(!is_discovered_loopback_printer(
            "tcp://127.0.0.1:8069",
            &discovered
        ));
        // reject_metadata_resolution rejects IPv4-mapped metadata and fails closed on DNS errors.
        assert_eq!(
            NativePrintBackend::reject_metadata_resolution("::ffff:169.254.169.254", 9100)
                .unwrap_err()
                .code,
            "BAD_PRINTER_ADDRESS"
        );
        assert_eq!(
            NativePrintBackend::reject_metadata_resolution("nonexistent.invalid", 9100)
                .unwrap_err()
                .code,
            "DNS_RESOLUTION_FAILED"
        );
    }

    #[test]
    fn deserialize_bytes_rejects_oversized_map() {
        #[derive(Deserialize)]
        struct Wrap {
            #[serde(deserialize_with = "deserialize_bytes")]
            bytes: Vec<u8>,
        }
        let ok: Wrap = serde_json::from_str(r#"{"bytes":{"0":37,"1":80}}"#).unwrap();
        assert_eq!(ok.bytes, vec![37, 80]);

        let oversized = format!(r#"{{"bytes":{{"{}":1}}}}"#, MAX_BYTE_MAP_LEN);
        let err = serde_json::from_str::<Wrap>(&oversized).err().unwrap();
        assert!(
            err.to_string().contains("payload_too_large"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_paper_width_mm_and_case_insensitive_uri() {
        assert_eq!(parse_paper_width_mm(Some("80mm")), Some(80.0));
        assert_eq!(parse_paper_width_mm(Some("58mm x 210mm")), Some(58.0));
        assert_eq!(parse_paper_width_mm(Some("A4")), None);
        assert_eq!(parse_paper_width_mm(Some("210mm")), None);
        let parsed = NativePrintBackend::parse_host_port("TCP://192.168.1.50:9100", 9100).unwrap();
        assert_eq!(
            parsed,
            ("192.168.1.50".to_string(), 9100, "/ipp/print".to_string())
        );

        let mut list = vec![PrinterInfo {
            name: "Zebra ZPL Lab".into(),
            kind: PrinterKind::Local,
            is_default: false,
            port: "127.0.0.1:9101".into(),
            driver: "Generic / Text Only".into(),
            status: print_core::PrinterStatus::Ready,
            protocol: "zpl".into(),
            printer_class: "label".into(),
            stable_id: String::new(),
            health: None,
        }];
        reconcile_default_flag(&mut list, Some("Zebra ZPL Lab"));
        assert!(list[0].is_default);

        let generic_caps = print_core::PrinterCapabilities {
            resolutions_dpi: vec![600],
            forms: vec![print_core::Form {
                name: "Ledger".into(),
                size: print_core::PageSize {
                    width_mm: 279.4,
                    height_mm: 431.8,
                },
                roll: false,
                paper_id: Some(4),
            }],
            ..Default::default()
        };
        let (box_dots, dpi) = resolve_label_box_and_dpi(&generic_caps);
        assert_eq!(dpi, 203);
        assert_eq!(box_dots, Some((815, 1215)));
    }
}

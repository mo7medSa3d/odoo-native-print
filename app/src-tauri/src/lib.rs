mod commands;
mod config;

use commands::{is_local_app_url, spawn_worker_loop, AppState};
use print_core::security::origin_is_allowed;
use print_core::Queue;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// Bridge injected into the webview. Maps `window.__NATIVE_PRINT__` calls
/// onto Tauri IPC commands (which re-validate the caller's origin in Rust
/// before touching the queue), normalizes `Uint8Array` / `ArrayBuffer`
/// byte payloads into standard arrays for serde, and registers the
/// `Ctrl+Shift+P` shortcut to toggle between Odoo and the Native Print
/// Control Center.
const BRIDGE_JS: &str = r#"
(function () {
  function normalizeBytes(b) {
    if (!b) return [];
    if (Array.isArray(b) || typeof b === "string") return b;
    if (b instanceof ArrayBuffer) return Array.from(new Uint8Array(b));
    if (ArrayBuffer.isView(b)) return Array.from(new Uint8Array(b.buffer, b.byteOffset, b.byteLength));
    return b;
  }
  function invoke(cmd, args) {
    return window.__TAURI_INTERNALS__.invoke(cmd, args || {});
  }
  window.__NATIVE_PRINT__ = {
    version: "__APP_VERSION__",
    printPdf: function (args) {
      const payload = Object.assign({}, args, { bytes: normalizeBytes(args && args.bytes) });
      return invoke("print_pdf", { req: payload });
    },
    printText: function (args) {
      return invoke("print_text", { req: args });
    },
    printImage: function (args) {
      const payload = Object.assign({}, args, { bytes: normalizeBytes(args && args.bytes) });
      return invoke("print_image", { req: payload });
    },
    printStatus: function (jobId) {
      return invoke("print_status", { jobId: jobId, job_id: jobId });
    },
    jobDetail: function (jobId) {
      return invoke("job_detail", { jobId: jobId, job_id: jobId });
    },
    appOverview: function () {
      return invoke("app_overview", {});
    },
    listPrinters: function () {
      return invoke("list_printers", {});
    },
    printerForms: function (printer) {
      return invoke("printer_forms", { printer: printer });
    },
    printerCapabilities: function (printer) {
      return invoke("printer_capabilities", { printer: printer });
    },
    runTestPrint: function (printer, kind, paper) {
      return invoke("run_test_print", { printer: printer, kind: kind, paper: paper || null });
    },
    retryJob: function (jobId) {
      return invoke("retry_job_cmd", { jobId: jobId, job_id: jobId });
    },
    cancelJob: function (jobId) {
      return invoke("cancel_job_cmd", { jobId: jobId, job_id: jobId });
    },
    clearFinished: function () {
      return invoke("clear_finished_cmd", {});
    },
    exportDiagnostics: function () {
      return invoke("export_diagnostics", {});
    },
    saveOrigin: function (origin, autoReprint) {
      return invoke("save_origin_cmd", {
        origin: origin,
        autoReprint: !!autoReprint,
        auto_reprint: !!autoReprint
      });
    },
    openControlCenter: function () {
      return invoke("open_control_center", {});
    },
    openOdooOrigin: function () {
      return invoke("open_odoo_origin", {});
    },
  };
  window.addEventListener("keydown", function (ev) {
    if (ev.ctrlKey && ev.shiftKey && (ev.key === "P" || ev.key === "p")) {
      ev.preventDefault();
      if (window.location.hostname === "tauri.localhost" || window.location.protocol === "tauri:") {
        window.__NATIVE_PRINT__.openOdooOrigin().catch(function () {});
      } else {
        window.__NATIVE_PRINT__.openControlCenter().catch(function () {});
      }
    }
  });
})();
"#;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&config_dir)?;
            let resolved = config::resolve_config(&config_dir);

            // Durable queue in the per-user data dir (RDS/Citrix safe:
            // nothing is shared between sessions).
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir
                .join("print-queue.db")
                .to_string_lossy()
                .to_string();
            let mut queue =
                Queue::open(&db_path).map_err(|e| std::io::Error::other(e.to_string()))?;
            // Crash recovery per ADR-0004.
            let _ = queue.recover_on_startup(resolved.auto_reprint);

            let queue_arc = Arc::new(Mutex::new(queue));
            let worker_wake = Arc::new((Mutex::new(true), Condvar::new()));
            spawn_worker_loop(db_path.clone(), Arc::clone(&worker_wake));

            let initial_origin = resolved.odoo_origin.clone();
            app.manage(AppState {
                queue: queue_arc,
                db_path,
                configured_origin: RwLock::new(initial_origin.clone()),
                config_dir,
                locked_by_policy: resolved.locked_by_policy,
                auto_reprint: RwLock::new(resolved.auto_reprint),
                worker_wake,
            });

            // If a valid HTTP IP Odoo origin is configured, open it directly;
            // otherwise open the built-in local Control Center (`index.html`)
            // so the administrator can configure the origin or run printer
            // diagnostics without the app crashing on startup.
            let start_url = match initial_origin.as_deref() {
                Some(origin) => match origin.parse::<tauri::Url>() {
                    Ok(u) => WebviewUrl::External(u),
                    Err(_) => WebviewUrl::App("index.html".into()),
                },
                None => WebviewUrl::App("index.html".into()),
            };

            let app_handle = app.handle().clone();
            WebviewWindowBuilder::new(app, "main", start_url)
                .title("Odoo Native Print")
                .inner_size(1280.0, 820.0)
                .min_inner_size(960.0, 640.0)
                .initialization_script(
                    BRIDGE_JS.replace("__APP_VERSION__", env!("CARGO_PKG_VERSION")),
                )
                .on_navigation(move |u| {
                    if u.as_str() == "about:blank" || is_local_app_url(u) {
                        return true;
                    }
                    let state = app_handle.state::<AppState>();
                    let allowed = state.configured_origin.read().ok().and_then(|g| g.clone());
                    match allowed {
                        Some(cfg_origin) => {
                            let o = u.origin().ascii_serialization();
                            origin_is_allowed(&cfg_origin, &o)
                        }
                        None => false,
                    }
                })
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::print_pdf,
            commands::print_text,
            commands::print_image,
            commands::print_status,
            commands::job_detail,
            commands::app_overview,
            commands::list_printers,
            commands::printer_forms,
            commands::printer_capabilities,
            commands::run_test_print,
            commands::retry_job_cmd,
            commands::cancel_job_cmd,
            commands::clear_finished_cmd,
            commands::export_diagnostics,
            commands::save_origin_cmd,
            commands::open_control_center,
            commands::open_odoo_origin
        ])
        .run(tauri::generate_context!())
        .expect("error while running Odoo Native Print");
}

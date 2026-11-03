mod commands;
mod config;

use commands::AppState;
use print_core::security::origin_is_allowed;
use print_core::Queue;
use std::sync::Mutex;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// Bridge injected ONLY into the configured Odoo origin's page. It maps
/// the addon's window.__NATIVE_PRINT__ calls onto Tauri IPC commands,
/// which re-validate the origin in Rust before doing anything.
const BRIDGE_JS: &str = r#"
window.__NATIVE_PRINT__ = {
  printPdf: (args) => window.__TAURI_INTERNALS__.invoke("print_pdf", { req: args }),
  printText: (args) => window.__TAURI_INTERNALS__.invoke("print_text", { req: args }),
  printImage: (args) => window.__TAURI_INTERNALS__.invoke("print_image", { req: args }),
  printStatus: (jobId) => window.__TAURI_INTERNALS__.invoke("print_status", { jobId }),
};
"#;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&config_dir)?;
            let origin = config::load_origin(&config_dir)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

            // Durable queue in the per-user data dir (RDS/Citrix safe:
            // nothing is shared between sessions).
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let mut queue = Queue::open(data_dir.join("print-queue.db").to_str().unwrap())
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            // Crash recovery per ADR-0004 (safe default: no auto-reprint).
            let _ = queue.recover_on_startup(false);
            app.manage(AppState {
                queue: Mutex::new(queue),
                configured_origin: origin.clone(),
            });

            let url: tauri::Url = origin.parse().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad origin URL")
            })?;
            let guard_origin = origin.clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("Odoo Native Print")
                .inner_size(1280.0, 800.0)
                .initialization_script(BRIDGE_JS)
                .on_navigation(move |u| {
                    // Only the configured origin may load in this window.
                    // Anything else is blocked here (the shell offers no
                    // "open externally" for an enterprise kiosk-style app).
                    let o = u.origin().ascii_serialization();
                    u.as_str() == "about:blank" || origin_is_allowed(&guard_origin, &o)
                })
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::print_pdf,
            commands::print_text,
            commands::print_image,
            commands::print_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running Odoo Native Print");
}

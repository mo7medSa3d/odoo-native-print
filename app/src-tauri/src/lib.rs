mod commands;
mod config;

use commands::{is_local_app_url, spawn_worker_loop, AppState};
use print_core::security::origin_is_allowed;
use print_core::Queue;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use tauri::{
    LogicalPosition, LogicalSize, Manager, WebviewBuilder, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};

/// Bridge injected into the webview. Maps `window.__NATIVE_PRINT__` calls
/// onto Tauri IPC commands (which re-validate the caller's origin in Rust
/// before touching the queue), normalizes `Uint8Array` / `ArrayBuffer`
/// byte payloads into standard arrays for serde, and registers the
/// `Ctrl+Shift+Y` shortcut to toggle between Odoo and the Yaseir Print
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
  // Page error tap. This script runs before any page script, so a
  // bundle that dies on load (a frozen-prototype TypeError, a syntax
  // error on a stale WebView2 runtime, a failed asset) is captured
  // here instead of leaving a silent white page. Captured errors are
  // surfaced in the boot card, the environment report and the
  // diagnostics bundle.
  window.__NP_ERRORS__ = [];
  function npPushError(kind, message) {
    try {
      var list = window.__NP_ERRORS__;
      if (list.length >= 10) return;
      var text = String(kind || "error") + ": " + String(message == null ? "" : message).slice(0, 300);
      if (list.indexOf(text) !== -1) return;
      list.push(text);
      if (window.__NP_ERROR_TIMER__) return;
      window.__NP_ERROR_TIMER__ = window.setTimeout(function () {
        window.__NP_ERROR_TIMER__ = null;
        npReportEnvironment();
      }, 300);
    } catch (e) { /* the tap must never break the page */ }
  }
  window.addEventListener("error", function (ev) {
    var t = ev && ev.target;
    if (t && t !== window && (t.tagName === "SCRIPT" || t.tagName === "LINK")) {
      npPushError("load", (t.src || t.href || "asset") + " failed to load");
    } else if (ev && ev.message) {
      npPushError("error", ev.message);
    }
  }, true);
  window.addEventListener("unhandledrejection", function (ev) {
    var r = ev && ev.reason;
    npPushError("promise", (r && (r.message || r)) || "unhandled rejection");
  });
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
    reportEnvironment: function (report) {
      return invoke("report_environment", { report: report });
    },
    clearWebData: function () {
      return invoke("clear_web_data_cmd", {});
    },
  };

  // ------------------------------------------------------------------
  // Environment probes + boot watchdog. The Odoo frontend is a large
  // JS application: if the webview environment cannot run it (stale
  // WebView2 runtime, blocked storage, poisoned profile), the result
  // must never be a silent white page. The probes record the facts;
  // the watchdog turns a failed boot into a visible, actionable
  // panel with working recovery actions.
  // ------------------------------------------------------------------
  function npIsLocalPage() {
    return window.location.hostname === "tauri.localhost" || window.location.protocol === "tauri:";
  }
  // Unencrypted-origin warning: HTTP to an IP address is allowed by
  // the owner's policy (ADR-0002), but credentials and documents cross
  // the network unencrypted. A slim strip says so on every HTTP page
  // until dismissed for the session; it never blocks work.
  function npHttpWarning() {
    if (window.location.protocol !== 'http:') { return; }
    if (npIsLocalPage()) { return; }
    try {
      if (window.sessionStorage.getItem('np_http_warned') === '1') { return; }
    } catch (e) { /* storage blocked: still warn */ }
    var show = function () {
      if (document.getElementById('np-http-warning')) { return; }
      var bar = document.createElement('div');
      bar.id = 'np-http-warning';
      bar.setAttribute('style', 'position:fixed;top:0;left:0;right:0;z-index:2147483646;background:#8a6d00;color:#fff;font:13px/1.5 system-ui,sans-serif;padding:6px 12px;display:flex;justify-content:space-between;align-items:center;direction:rtl;');
      var text = document.createElement('span');
      text.textContent = 'تنبيه: الاتصال بخادم Odoo غير مشفّر (HTTP) — بيانات الدخول والمستندات تمر دون تشفير على الشبكة.';
      var btn = document.createElement('button');
      btn.textContent = 'حسنًا';
      btn.setAttribute('style', 'background:transparent;border:1px solid #fff;color:#fff;border-radius:999px;padding:2px 14px;cursor:pointer;font:inherit;');
      btn.addEventListener('click', function () {
        try { window.sessionStorage.setItem('np_http_warned', '1'); } catch (e) {}
        bar.remove();
      });
      bar.appendChild(text);
      bar.appendChild(btn);
      document.body.appendChild(bar);
    };
    if (document.body) { show(); }
    else { document.addEventListener('DOMContentLoaded', show); }
  }

  function npProbeEnvironment() {
    var env = {
      userAgent: navigator.userAgent || "",
      href: window.location.href,
      secureContext: window.isSecureContext === true,
      online: navigator.onLine !== false,
    };
    try {
      env.es2022 = typeof Array.prototype.at === "function" && typeof Object.hasOwn === "function";
    } catch (e) { env.es2022 = false; }
    try { env.structuredClone = typeof structuredClone === "function"; } catch (e) { env.structuredClone = false; }
    try { env.randomUUID = !!(window.crypto && typeof window.crypto.randomUUID === "function"); } catch (e) { env.randomUUID = false; }
    try {
      window.localStorage.setItem("__np_probe__", "1");
      window.localStorage.removeItem("__np_probe__");
      env.localStorage = true;
    } catch (e) { env.localStorage = false; }
    try {
      window.sessionStorage.setItem("__np_probe__", "1");
      window.sessionStorage.removeItem("__np_probe__");
      env.sessionStorage = true;
    } catch (e) { env.sessionStorage = false; }
    try { env.cookies = navigator.cookieEnabled === true; } catch (e) { env.cookies = false; }
    try { env.webSocket = typeof WebSocket === "function"; } catch (e) { env.webSocket = false; }
    try { env.errors = (window.__NP_ERRORS__ || []).slice(0, 10); } catch (e) { env.errors = []; }
    return env;
  }
  function npReportEnvironment() {
    try {
      var env = npProbeEnvironment();
      window.__NATIVE_PRINT_ENV__ = env;
      invoke("report_environment", { report: JSON.stringify(env) }).catch(function () {});
    } catch (e) { /* probes must never break the page */ }
  }
  function npIsBackendPage() {
    var p = window.location.pathname || "/";
    if (p === "/odoo" || p.indexOf("/odoo/") === 0) return true;
    if (p === "/web" || p.indexOf("/web/") === 0) {
      return p.indexOf("/web/login") !== 0 && p.indexOf("/web/database") !== 0;
    }
    return false;
  }
  function npBooted() {
    // The web client shell existing at all means Odoo's app started.
    // Requiring children was a false-positive source: on a slow first
    // boot the shell can sit empty past the old deadline, and the
    // watchdog then covered a healthy page with a dead white screen.
    if (document.querySelector(".o_web_client")) return true;
    return !!document.querySelector(".o_action_manager, .o_main_content");
  }
  function npBootOverlay(mode, env) {
    var existing = document.getElementById("np-boot-overlay");
    if (mode === "hide") {
      if (existing) existing.remove();
      return;
    }
    if (!document.body) return;
    var el = existing;
    if (!el) {
      el = document.createElement("div");
      el.id = "np-boot-overlay";
      // A corner card, never a full-screen cover: the page behind it
      // stays visible and clickable, so even a false alarm costs the
      // user nothing.
      el.setAttribute("style", "position:fixed;right:16px;bottom:16px;z-index:2147483647;font-family:inherit;");
      document.body.appendChild(el);
    }
    function facts() {
      var rows = [];
      function row(label, ok) {
        return '<div style="display:flex;justify-content:space-between;gap:2rem;padding:0.3rem 0;border-bottom:1px solid #eef1f6;font-size:0.85rem;">' +
          '<span style="color:#5b6472;">' + label + '</span>' +
          '<strong style="color:' + (ok ? '#12713f' : '#b3261e') + ';">' + (ok ? 'OK' : 'غير متاح') + '</strong></div>';
      }
      rows.push(row('JavaScript حديث (ES2022)', !!env.es2022));
      rows.push(row('التخزين المحلي', !!env.localStorage));
      rows.push(row('تخزين الجلسة', !!env.sessionStorage));
      rows.push(row('ملفات تعريف الارتباط', !!env.cookies));
      rows.push(row('WebSocket', !!env.webSocket));
      rows.push(row('سياق آمن (HTTPS/IP)', !!env.secureContext));
      return rows.join("");
    }
    function errorRows() {
      var errs = [];
      try { errs = window.__NP_ERRORS__ || []; } catch (e) {}
      if (!errs.length) return "";
      var esc = function (x) { return String(x).replace(/&/g, "&amp;").replace(/</g, "&lt;"); };
      var html = '<div style="margin-top:0.8rem;"><div style="font-size:0.85rem;font-weight:700;color:#b3261e;margin-bottom:0.3rem;">أخطاء الصفحة:</div>';
      for (var i = 0; i < Math.min(errs.length, 3); i++) {
        html += '<div style="font-size:0.78rem;color:#5b6472;word-break:break-word;padding:0.15rem 0;">' + esc(errs[i]) + '</div>';
      }
      return html + "</div>";
    }
    var btnBase = 'border:0;border-radius:999px;padding:0.65rem 1.2rem;font-size:0.9rem;cursor:pointer;background:#eef1f6;color:#101219;';
    el.innerHTML = '<div style="max-width:460px;width:92vw;padding:1.4rem;background:#ffffff;border:1px solid #e3e8f0;border-radius:20px;box-shadow:0 12px 40px rgba(16,18,25,0.16);">' +
      '<div style="font-size:1.15rem;font-weight:800;color:#101219;margin-bottom:0.4rem;">واجهة Odoo لم تكتمل</div>' +
      '<div style="color:#5b6472;font-size:0.9rem;margin-bottom:1rem;">الصفحة وصلت لكن تطبيق Odoo لم يبدأ. دي نتيجة فحص بيئة العرض على الجهاز ده:</div>' +
      facts() +
      errorRows() +
      '<div style="display:flex;flex-wrap:wrap;gap:0.5rem;margin-top:1.2rem;">' +
      '<button id="np-boot-reload" style="' + btnBase + 'background:#0953DE;color:#fff;">إعادة التحميل</button>' +
      '<button id="np-boot-clear" style="' + btnBase + '">مسح بيانات التصفح والمحاولة من جديد</button>' +
      '<button id="np-boot-cc" style="' + btnBase + '">مركز التحكم</button>' +
      '</div>' +
      '<div style="color:#8a93a5;font-size:0.78rem;margin-top:0.9rem;">مسح البيانات بيخرّجك من الحساب ويمسح الكاش والتخزين المحلي للتطبيق فقط.</div></div>';
    var reload = document.getElementById("np-boot-reload");
    if (reload) reload.addEventListener("click", function () { window.location.reload(); });
    var clear = document.getElementById("np-boot-clear");
    if (clear) clear.addEventListener("click", function () {
      clear.disabled = true;
      clear.textContent = 'جارٍ المسح…';
      invoke("clear_web_data_cmd", {}).catch(function () { window.location.reload(); });
    });
    var cc = document.getElementById("np-boot-cc");
    if (cc) cc.addEventListener("click", function () {
      window.__NATIVE_PRINT__.openControlCenter().catch(function () {});
    });
  }
  function npBootWatchdog() {
    if (npIsLocalPage() || !npIsBackendPage()) return;
    var delays = window.__NP_BOOT_DELAYS__ || { tickMs: 1000, failAfter: 30 };
    var env = window.__NATIVE_PRINT_ENV__ || npProbeEnvironment();
    var ticks = 0;
    var timer = window.setInterval(function () {
      ticks += 1;
      if (npBooted()) {
        window.clearInterval(timer);
        npBootOverlay("hide", env);
        return;
      }
      // Show the diagnostic card once, then KEEP polling: a late boot
      // removes it automatically. There is deliberately no "loading"
      // cover any more — a slow boot is not a failed boot.
      if (ticks === delays.failAfter) { npReportEnvironment(); npBootOverlay("problem", npProbeEnvironment()); }
    }, delays.tickMs);
  }
  if (!npIsLocalPage()) {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", function () { npReportEnvironment(); npBootWatchdog(); npHttpWarning(); });
    } else {
      npReportEnvironment();
      npBootWatchdog();
      npHttpWarning();
    }
  }
  window.addEventListener("keydown", function (ev) {
    if (ev.altKey && (ev.key === "ArrowLeft" || ev.key === "ArrowRight")) {
      var t = ev.target;
      var editable = t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable);
      if (!editable) {
        ev.preventDefault();
        if (ev.key === "ArrowLeft") { window.history.back(); } else { window.history.forward(); }
      }
      return;
    }
    if (ev.ctrlKey && ev.shiftKey && (ev.key === "Y" || ev.key === "y")) {
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
            let environment = RwLock::new(None);
            let worker_wake = Arc::new((Mutex::new(true), Condvar::new()));
            spawn_worker_loop(db_path.clone(), Arc::clone(&worker_wake));

            let initial_origin = resolved.odoo_origin.clone();
            app.manage(AppState {
                queue: queue_arc,
                configured_origin: RwLock::new(initial_origin.clone()),
                config_dir,
                environment,
                printer_cache: RwLock::new(None),
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
            let window = WebviewWindowBuilder::new(app, "main", start_url)
                .title("Yaseir Print")
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

            // NOTE (v1.4.1): the child-webview toolbar block is
            // intentionally absent. Every variant of it broke the
            // Windows installer build (CI runs #68-#70, #73-#75)
            // while the identical tree without it built green
            // (run #71), and the CI logs are not reachable from the
            // build environment to extract the exact compiler
            // error. The nav commands (nav_back / nav_forward /
            // nav_reload), the toolbar capability and toolbar.html
            // stay in place, inert, and back/forward is available
            // in the Odoo view via Alt+Left / Alt+Right (bridge).
            // Restore the block only with a Windows compiler log
            // in hand (build once on a Windows machine and read
            // the first error).
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
            commands::report_environment,
            commands::clear_web_data_cmd,
            commands::save_origin_cmd,
            commands::open_control_center,
            commands::open_odoo_origin,
            commands::app_identity,
            commands::nav_back,
            commands::nav_forward,
            commands::nav_reload
        ])
        .run(tauri::generate_context!())
        .expect("error while running Yaseir Print");
}

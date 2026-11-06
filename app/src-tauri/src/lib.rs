mod commands;
mod config;

use commands::{is_local_app_url, spawn_worker_loop, AppState, TabsState};
use print_core::security::origin_is_allowed;
use print_core::Queue;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use tauri::{Manager, WebviewUrl};

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
      // If the boot card is already on screen, re-render it so the
      // newest (often most diagnostic) error is what the user sees.
      try {
        if (document.getElementById("np-boot-overlay")) {
          npBootOverlay("problem", npProbeEnvironment());
        }
      } catch (e) { /* ignore */ }
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
  // Subframes get no facade and no watchdog: a third-party frame
  // embedded in the Odoo page must not inherit the top page's print
  // rights or UI overlays (audit B2). The error tap above stays in
  // every frame; it is read-only diagnostics.
  var __npIsTop = false;
  try { __npIsTop = window === window.top; } catch (e) { __npIsTop = true; }
  const __npFacade = {
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
      return invoke("print_status", { jobId: jobId });
    },
    jobDetail: function (jobId) {
      return invoke("job_detail", { jobId: jobId });
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
    runTestPrint: function (printer, kind, paper, options) {
      // Test prints are a Control Center feature (Rust enforces
      // local-only too): never expose them to remote Odoo pages.
      if (!npIsLocalPage()) {
        return Promise.reject(new Error("runTestPrint is only available in the Control Center"));
      }
      var o = options || {};
      return invoke("run_test_print", {
        printer: printer, kind: kind, paper: paper || null,
        options: {
          tray: o.tray || null,
          duplex: typeof o.duplex === "boolean" ? o.duplex : null,
          color: typeof o.color === "boolean" ? o.color : null
        }
      });
    },
    retryJob: function (jobId) {
      return invoke("retry_job_cmd", { jobId: jobId });
    },
    cancelJob: function (jobId) {
      return invoke("cancel_job_cmd", { jobId: jobId });
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
        autoReprint: !!autoReprint
      });
    },
    getPrinterSettings: function () {
      // Per-printer defaults are a Control Center feature (Rust
      // enforces local-only too): remote Odoo pages never see them.
      if (!npIsLocalPage()) {
        return Promise.reject(new Error("printer settings are only available in the Control Center"));
      }
      return invoke("get_printer_settings", {});
    },
    setPrinterSettings: function (printer, settings) {
      if (!npIsLocalPage()) {
        return Promise.reject(new Error("printer settings are only available in the Control Center"));
      }
      return invoke("set_printer_settings", { printer: printer, settings: settings });
    },
    openControlCenter: function () {
      return invoke("open_control_center", {});
    },
    openTab: function (url) {
      // New tab on the configured origin (Rust validates the URL
      // against the configured origin; off-origin is refused).
      return invoke("tab_open", { url: String(url) });
    },
    openOdooOrigin: function () {
      return invoke("open_odoo_origin", {});
    },
    reportEnvironment: function (report) {
      return invoke("report_environment", { report: report });
    },
    clearWebData: function () {
      // Wiping browsing data logs the user out, so it is a Control
      // Center action only (the capability grant and the Rust check
      // enforce local-only too): a remote page must never be able to
      // trigger it, let alone loop it into a forced logout (audit M2).
      if (!npIsLocalPage()) {
        return Promise.reject(new Error("clearWebData is only available in the Control Center"));
      }
      return invoke("clear_web_data_cmd", {});
    },
  };
  // Publish the facade as a non-writable, non-configurable property:
  // page scripts (including third-party Odoo JS) must not be able to
  // replace or wrap it to intercept payloads or fake results. The
  // fallback assignment covers a page that somehow pre-defined the
  // name as configurable. (Audit B-6.)
  if (__npIsTop) {
    try {
      Object.defineProperty(window, "__NATIVE_PRINT__", {
        value: __npFacade,
        writable: false,
        configurable: false,
      });
    } catch (e) {
      window.__NATIVE_PRINT__ = __npFacade;
    }
  }

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
      bar.setAttribute('style', 'position:fixed;bottom:0;left:0;right:0;z-index:2147483646;background:#8a6d00;color:#fff;font:13px/1.5 system-ui,sans-serif;padding:6px 12px;display:flex;justify-content:space-between;align-items:center;direction:rtl;');
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
    // The user dismissed the card for this page (audit JS-M4): a
    // healthy-but-unusual view the content check does not recognize
    // must not be overlaid forever. The watchdog keeps running; a
    // full navigation re-arms the card on the next page.
    if (window.__NP_CARD_DISMISSED__) return;
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
    function serverHint() {
      var errs = [];
      try { errs = window.__NP_ERRORS__ || []; } catch (e) {}
      var isTranslations = false;
      for (var i = 0; i < errs.length; i++) {
        if (/fetching translations/i.test(errs[i])) { isTranslations = true; break; }
      }
      var origin = "";
      try { origin = window.location.origin; } catch (e) {}
      var esc = function (x) { return String(x).replace(/&/g, "&amp;").replace(/</g, "&lt;"); };
      var html = '<div style="font-size:0.85rem;color:#5b6472;margin-bottom:0.8rem;">الخادم المستهدف: <span dir="ltr" style="font-weight:600;">' + esc(origin) + '</span></div>';
      if (isTranslations) {
        html += '<div style="background:#fdf3e3;border:1px solid #f0d9a8;border-radius:12px;padding:0.7rem 0.9rem;font-size:0.85rem;color:#6b5410;margin-bottom:0.8rem;">السبب المرجّح: سيرفر Odoo رجّع خطأ في ملف الترجمة — المشكلة في السيرفر مش في الجهاز ده. الحل: حدّث إضافة الطباعة (native_print) على السيرفر لآخر إصدار وأعد تشغيل Odoo.</div>';
      }
      return html;
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
    el.innerHTML = '<div style="position:relative;max-width:460px;width:92vw;padding:1.4rem;background:#ffffff;border:1px solid #e3e8f0;border-radius:20px;box-shadow:0 12px 40px rgba(16,18,25,0.16);">' +
      '<button id="np-boot-dismiss" title="إخفاء" style="position:absolute;top:10px;inset-inline-end:12px;border:0;background:transparent;color:#8a93a5;font-size:1.05rem;line-height:1;cursor:pointer;padding:4px 6px;">✕</button>' +
      '<div style="font-size:1.15rem;font-weight:800;color:#101219;margin-bottom:0.4rem;">واجهة Odoo لم تكتمل</div>' +
      '<div style="color:#5b6472;font-size:0.9rem;margin-bottom:1rem;">الصفحة وصلت لكن تطبيق Odoo لم يبدأ. دي نتيجة فحص بيئة العرض على الجهاز ده:</div>' +
      serverHint() +
      facts() +
      errorRows() +
      '<div style="display:flex;flex-wrap:wrap;gap:0.5rem;margin-top:1.2rem;">' +
      '<button id="np-boot-reload" style="' + btnBase + 'background:#0953DE;color:#fff;">إعادة التحميل</button>' +
      '<button id="np-boot-clear" style="' + btnBase + '">مسح بيانات التصفح من مركز التحكم</button>' +
      '<button id="np-boot-cc" style="' + btnBase + '">مركز التحكم</button>' +
      '</div>' +
      '<div style="color:#8a93a5;font-size:0.78rem;margin-top:0.9rem;">لو إعادة التحميل ما نفعش، امسح بيانات التصفح من مركز التحكم (بيخرّجك من الحساب ويمسح الكاش والتخزين المحلي للتطبيق فقط).</div></div>';
    var reload = document.getElementById("np-boot-reload");
    if (reload) reload.addEventListener("click", function () { window.location.reload(); });
    var dismiss = document.getElementById("np-boot-dismiss");
    if (dismiss) dismiss.addEventListener("click", function () {
      window.__NP_CARD_DISMISSED__ = true;
      el.remove();
    });
    var clear = document.getElementById("np-boot-clear");
    if (clear) clear.addEventListener("click", function () {
      // The wipe itself is Control-Center-only (it logs the user
      // out; a remote page must not trigger it — audit M2). The card
      // takes the user there instead of invoking it directly.
      window.__NATIVE_PRINT__.openControlCenter().catch(function () {});
    });
    var cc = document.getElementById("np-boot-cc");
    if (cc) cc.addEventListener("click", function () {
      window.__NATIVE_PRINT__.openControlCenter().catch(function () {});
    });
  }
  function npContentReady() {
    // Second stage: the shell alone is not a boot — the initial
    // action must render content, otherwise the user is staring at
    // an empty (white) content area under a live shell. POS renders
    // outside the action manager; its shell is accepted as before.
    try {
      if ((window.location.pathname || "").indexOf("/pos/") !== -1) return true;
    } catch (e) {}
    var am = document.querySelector(".o_action_manager");
    if (am && am.children.length > 0) return true;
    return !!document.querySelector(".o-mail-Discuss, .o_mail_discuss, .o_kanban_view, .o_list_view, .o_form_view, .o_pivot_view, .o_graph_view, .o_calendar_view");
  }
  function npBootWatchdog() {
    if (window.__NP_WATCHDOG_ARMED__) return;
    if (npIsLocalPage() || !npIsBackendPage()) return;
    window.__NP_WATCHDOG_ARMED__ = true;
    var delays = window.__NP_BOOT_DELAYS__ || { tickMs: 1000, failAfter: 30 };
    var env = window.__NATIVE_PRINT_ENV__ || npProbeEnvironment();
    var ticks = 0;
    var shellAt = 0;
    var cardShown = false;
    var timer = window.setInterval(function () {
      ticks += 1;
      // The same document can soft-redirect to the login page
      // (session expiry): that page is healthy, never flag it.
      if (!npIsBackendPage()) {
        window.clearInterval(timer);
        window.__NP_WATCHDOG_ARMED__ = false;
        npBootOverlay("hide", env);
        return;
      }
      if (npBooted()) {
        if (!shellAt) shellAt = ticks;
        if (npContentReady()) {
          window.clearInterval(timer);
          // Re-armable: later SPA route changes re-run the check
          // via the pushState/popstate hooks below.
          window.__NP_WATCHDOG_ARMED__ = false;
          npBootOverlay("hide", env);
          return;
        }
        // Shell up, content still empty after a further grace
        // window: the white-content failure mode.
        if (!cardShown && ticks - shellAt >= 20) {
          cardShown = true;
          npReportEnvironment();
          npBootOverlay("problem", npProbeEnvironment());
        }
        return;
      }
      // Show the diagnostic card once, then KEEP polling: a late boot
      // removes it automatically. There is deliberately no "loading"
      // cover any more — a slow boot is not a failed boot.
      if (!cardShown && ticks === delays.failAfter) { cardShown = true; npReportEnvironment(); npBootOverlay("problem", npProbeEnvironment()); }
    }, delays.tickMs);
  }
  if (__npIsTop && !npIsLocalPage()) {
    // Arm IMMEDIATELY at init-script evaluation: a page whose
    // DOMContentLoaded stalls (a hung render-blocking asset) still
    // gets its watchdog and environment report. Only the HTTP strip
    // and the nav pill need document.body, and they wait for it
    // themselves.
    npReportEnvironment();
    npBootWatchdog();
    npHttpWarning();
    // No floating nav pill any more: navigation lives in the
    // window's tab bar (owner brief, 2026-11-05).
    // Odoo is an SPA: it soft-navigates between actions and can
    // soft-navigate from a non-backend landing page into /odoo
    // without a new document load. Re-check on history transitions;
    // npBootWatchdog self-guards (armed flag, backend path, content
    // health) and re-arms after each successful boot.
    try {
      var npPushState = window.history.pushState;
      window.history.pushState = function () {
        var r = npPushState.apply(this, arguments);
        window.setTimeout(npBootWatchdog, 0);
        return r;
      };
      window.addEventListener("popstate", function () { npBootWatchdog(); });
    } catch (e) { /* history patching is best-effort */ }
  }
  window.addEventListener("keydown", function (ev) {
    if (window !== window.top) return;
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

/// Height of the tab bar strip at the top of the window (logical
/// pixels). Content webviews live below it.
pub const TABBAR_HEIGHT: f64 = 46.0;

/// Fit the tab bar and the ACTIVE content webview to the window's
/// current client size. Called at startup and on every resize.
pub fn layout_webviews(app: &tauri::AppHandle) {
    let Some(window) = app.get_window("main") else {
        return;
    };
    let Ok(size) = window.inner_size() else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let width = size.width as f64 / scale;
    let height = size.height as f64 / scale;
    if let Some(tabbar) = app.get_webview("tabbar") {
        let _ = tabbar.set_position(tauri::LogicalPosition::new(0.0, 0.0));
        let _ = tabbar.set_size(tauri::LogicalSize::new(width, TABBAR_HEIGHT));
    }
    let active = {
        let state = app.state::<AppState>();
        let Ok(tabs) = state.tabs.read() else { return };
        tabs.active.clone()
    };
    if let Some(label) = active {
        if let Some(wv) = app.get_webview(&label) {
            let _ = wv.set_position(tauri::LogicalPosition::new(0.0, TABBAR_HEIGHT));
            let _ = wv.set_size(tauri::LogicalSize::new(
                width,
                (height - TABBAR_HEIGHT).max(80.0),
            ));
        }
    }
}

/// Make `label` the visible tab: hide every other content webview,
/// show this one, refit it to the window.
pub fn activate_tab(app: &tauri::AppHandle, label: &str) {
    let labels = {
        let state = app.state::<AppState>();
        let Ok(mut tabs) = state.tabs.write() else {
            return;
        };
        if !tabs.order.iter().any(|l| l == label) {
            return;
        }
        tabs.active = Some(label.to_string());
        tabs.order.clone()
    };
    for other in &labels {
        if let Some(wv) = app.get_webview(other) {
            if other == label {
                let _ = wv.show();
            } else {
                let _ = wv.hide();
            }
        }
    }
    layout_webviews(app);
}

/// Create one content webview (a tab) in the main window. All tabs
/// share ONE data directory, so cookies and sessions are shared
/// exactly like tabs in a normal browser — which is how Odoo's
/// session model expects to be browsed (owner brief, 2026-11-05).
/// The navigation gate is the same origin policy as the old single
/// webview, plus the tab rules: POS navigations from a non-POS tab
/// are diverted into a fresh POS tab, and on-origin window.open
/// becomes a tab instead of being dropped.
pub fn create_tab_webview(
    app: &tauri::AppHandle,
    label: &str,
    url: WebviewUrl,
) -> Result<tauri::Webview, String> {
    let window = app
        .get_window("main")
        .ok_or_else(|| "main_window_missing".to_string())?;
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let nav_handle = app.clone();
    let nav_label = label.to_string();
    let open_handle = app.clone();
    let load_handle = app.clone();
    let load_label = label.to_string();
    let builder = tauri::webview::WebviewBuilder::new(label, url)
        .data_directory(data_dir)
        .initialization_script(BRIDGE_JS.replace("__APP_VERSION__", env!("CARGO_PKG_VERSION")))
        .on_navigation(move |u| {
            if u.as_str() == "about:blank" || is_local_app_url(u) {
                return true;
            }
            let state = nav_handle.state::<AppState>();
            let allowed = state.configured_origin.read().ok().and_then(|g| g.clone());
            let verdict = match allowed {
                Some(cfg_origin) => {
                    let o = u.origin().ascii_serialization();
                    origin_is_allowed(&cfg_origin, &o)
                }
                None => false,
            };
            if !verdict {
                eprintln!(
                    "Yaseir Print: blocked navigation to {} (not the configured Odoo origin)",
                    u.as_str()
                );
                return false;
            }
            // POS rule: entering the Point of Sale from any other
            // tab opens it in its own tab (it takes over the screen
            // and runs its own session logic; the backend tab the
            // user was on stays untouched behind it).
            if commands::is_pos_url(u.as_str()) {
                let tabs = state.tabs.read().ok();
                let is_pos_tab = tabs
                    .as_ref()
                    .map(|t| t.is_pos_tab(&nav_label))
                    .unwrap_or(false);
                let current_is_local = tabs
                    .as_ref()
                    .and_then(|t| t.url_of(&nav_label).map(|s| s.to_string()))
                    .map(|s| {
                        tauri::Url::parse(&s)
                            .map(|cu| is_local_app_url(&cu))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if !is_pos_tab && !current_is_local {
                    let spawn_handle = nav_handle.clone();
                    let target = u.clone();
                    tauri::async_runtime::spawn(async move {
                        let st = spawn_handle.state::<AppState>();
                        let _ = commands::open_tab_internal(&spawn_handle, &st, target).await;
                    });
                    return false;
                }
            }
            true
        })
        .on_new_window(move |u, _features| {
            // On-origin popups become tabs; anything else is denied
            // (the origin policy leaves the app no other browser).
            let state = open_handle.state::<AppState>();
            let allowed = state.configured_origin.read().ok().and_then(|g| g.clone());
            let on_origin = is_local_app_url(&u)
                || allowed
                    .map(|cfg| origin_is_allowed(&cfg, &u.origin().ascii_serialization()))
                    .unwrap_or(false);
            if on_origin {
                let spawn_handle = open_handle.clone();
                tauri::async_runtime::spawn(async move {
                    let st = spawn_handle.state::<AppState>();
                    let _ = commands::open_tab_internal(&spawn_handle, &st, u).await;
                });
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .on_page_load(move |_webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                if let Ok(mut tabs) = load_handle.state::<AppState>().tabs.write() {
                    tabs.set_url(&load_label, payload.url().as_str());
                }
            }
        });
    let (width, height) = {
        let scale = window.scale_factor().unwrap_or(1.0);
        match window.inner_size() {
            Ok(size) => (size.width as f64 / scale, size.height as f64 / scale),
            Err(_) => (1280.0, 820.0),
        }
    };
    window
        .add_child(
            builder,
            tauri::LogicalPosition::new(0.0, TABBAR_HEIGHT),
            tauri::LogicalSize::new(width, (height - TABBAR_HEIGHT).max(80.0)),
        )
        .map_err(|e| e.to_string())
}

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
            if let Err(e) = queue.recover_on_startup(resolved.auto_reprint) {
                eprintln!("Yaseir Print: startup queue recovery failed: {e}");
            }

            let queue_arc = Arc::new(Mutex::new(queue));
            let environment = RwLock::new(None);
            let worker_wake = Arc::new((Mutex::new(true), Condvar::new()));
            spawn_worker_loop(
                db_path.clone(),
                config_dir.clone(),
                Arc::clone(&worker_wake),
            );

            let initial_origin = resolved.odoo_origin.clone();
            app.manage(AppState {
                queue: queue_arc,
                configured_origin: RwLock::new(initial_origin.clone()),
                config_dir,
                environment,
                printer_cache: RwLock::new(None),
                tabs: RwLock::new(TabsState::default()),
                locked_by_policy: resolved.locked_by_policy,
                auto_reprint: RwLock::new(resolved.auto_reprint),
                worker_wake,
            });

            // If a valid HTTP IP Odoo origin is configured, open it directly;
            // otherwise open the built-in local Control Center (`index.html`)
            // so the administrator can configure the origin or run printer
            // diagnostics without the app crashing on startup.
            // With an origin configured, start on the local boot page:
            // it probes the origin (probe_origin) and only then
            // navigates the webview to Odoo, so a down or unreachable
            // server shows a styled local retry screen instead of
            // stranding the user on a WebView2 navigation-error page
            // (where no bridge, watchdog or shortcut exists).
            let start_url = match initial_origin.as_deref() {
                Some(_) => WebviewUrl::App("boot.html".into()),
                None => WebviewUrl::App("index.html".into()),
            };

            // Tabbed browser (owner brief, 2026-11-05): ONE window
            // hosting a local tab bar (tabbar.html) plus one child
            // webview per tab. This supersedes the v1.4.1 caution
            // below in history: the multi-webview API is enabled
            // (tauri "unstable" feature, already on), webviews are
            // created from setup or async commands only (the
            // documented Windows deadlock rule), and the Windows
            // build is verified on CI for this exact tree.
            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("Yaseir Print")
                .inner_size(1280.0, 820.0)
                .min_inner_size(960.0, 640.0)
                .build()?;
            let _tabbar = window.add_child(
                tauri::webview::WebviewBuilder::new(
                    "tabbar",
                    WebviewUrl::App("tabbar.html".into()),
                ),
                tauri::LogicalPosition::new(0.0, 0.0),
                tauri::LogicalSize::new(1280.0, TABBAR_HEIGHT),
            )?;

            // The first tab: the boot page (origin configured) or
            // the Control Center (nothing configured yet).
            {
                let state = app.state::<AppState>();
                let mut tabs = state
                    .tabs
                    .write()
                    .map_err(|_| std::io::Error::other("tabs_lock_poisoned"))?;
                tabs.next_id = 1;
                let first_url = match initial_origin.as_deref() {
                    Some(_) => "tauri://localhost/boot.html",
                    None => "tauri://localhost/index.html",
                };
                tabs.register("tab-1", first_url);
                tabs.active = Some("tab-1".to_string());
            }
            create_tab_webview(app.handle(), "tab-1", start_url)?;

            // Keep the tab bar and the active tab fitted to the
            // window on every resize.
            let resize_handle = app.handle().clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Resized(_)) {
                    layout_webviews(&resize_handle);
                }
            });
            layout_webviews(app.handle());
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
            commands::probe_origin,
            commands::retry_job_cmd,
            commands::cancel_job_cmd,
            commands::clear_finished_cmd,
            commands::export_diagnostics,
            commands::report_environment,
            commands::clear_web_data_cmd,
            commands::save_origin_cmd,
            commands::get_printer_settings,
            commands::set_printer_settings,
            commands::tab_open,
            commands::tab_list,
            commands::tab_switch,
            commands::tab_move,
            commands::tab_close,
            commands::open_control_center,
            commands::open_settings,
            commands::open_odoo_origin,
            commands::app_identity,
            commands::nav_back,
            commands::nav_forward,
            commands::nav_reload
        ])
        .run(tauri::generate_context!())
        .expect("error while running Yaseir Print");
}

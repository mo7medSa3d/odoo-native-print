mod commands;
mod config;

use commands::{is_local_app_url, spawn_worker_loop, AppState, TabsState};
use print_core::security::origin_is_allowed;
use print_core::Queue;
use std::sync::atomic::{AtomicBool, Ordering};
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
  if (window.__NP_BRIDGE_INSTALLED__) return;
  window.__NP_BRIDGE_INSTALLED__ = true;
  // Subframes get no facade, no listeners, no watchdog, and no
  // environment IPC: a third-party frame embedded in the Odoo page
  // must not inherit the top page's print rights or UI overlays
  // (audit B2). Fail closed if the top-frame check throws.
  var __npIsTop = false;
  try { __npIsTop = window === window.top; } catch (e) { __npIsTop = false; }
  if (!__npIsTop) return;
  function encodeBytesBase64(u8) {
    if (typeof btoa !== "function") return Array.from(u8);
    var chunks = [];
    for (var i = 0; i < u8.length; i += 0x8000) {
      chunks.push(String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000)));
    }
    return btoa(chunks.join(""));
  }
  function normalizeBytes(b) {
    if (!b) return [];
    if (Array.isArray(b) || typeof b === "string") return b;
    if (b instanceof ArrayBuffer) return encodeBytesBase64(new Uint8Array(b));
    if (ArrayBuffer.isView(b)) return encodeBytesBase64(new Uint8Array(b.buffer, b.byteOffset, b.byteLength));
    return b;
  }
  var __npTauriInternals = window.__TAURI_INTERNALS__;
  var __npTauriInvoke = __npTauriInternals && typeof __npTauriInternals.invoke === "function"
    ? __npTauriInternals.invoke.bind(__npTauriInternals)
    : null;
  function invoke(cmd, args) {
    if (__npTauriInvoke) return __npTauriInvoke(cmd, args || {});
    if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === "function") {
      return window.__TAURI_INTERNALS__.invoke(cmd, args || {});
    }
    return Promise.reject(new Error("tauri_internals_unavailable"));
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
      if (!__npIsTop) return;
      if (!window.__NP_ERROR_TIMER__) {
        window.__NP_ERROR_TIMER__ = window.setTimeout(function () {
          window.__NP_ERROR_TIMER__ = null;
          npReportEnvironment();
        }, 300);
      }
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
      npPushError("load", String(t.src || t.href || "asset").split(/[?#]/)[0] + " failed to load");
    } else if (ev && ev.message) {
      npPushError("error", ev.message);
    }
  }, true);
  window.addEventListener("unhandledrejection", function (ev) {
    var r = ev && ev.reason;
    npPushError("promise", (r && (r.message || r)) || "unhandled rejection");
  });
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
    appOverview: function (opts) {
      return invoke("app_overview", opts && opts.force ? { force: true } : {});
    },
    listPrinters: function (opts) {
      return invoke("list_printers", opts && opts.force ? { force: true } : {});
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
    try { Object.freeze(__npFacade); } catch (e) { /* ignore */ }
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

  // Environment probes + boot watchdog. The Odoo frontend is a large
  // JS application: if the webview environment cannot run it (stale
  // WebView2 runtime, blocked storage, poisoned profile), the result
  // must never be a silent white page. The probes record the facts;
  // the watchdog turns a failed boot into a visible, actionable
  // panel with working recovery actions.
  function npIsLocalPage() {
    if (window.location.port !== "") return false;
    var proto = window.location.protocol;
    var host = window.location.hostname;
    return (proto === "tauri:" && host === "localhost")
      || ((proto === "http:" || proto === "https:") && host === "tauri.localhost");
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
    else { document.addEventListener('DOMContentLoaded', show, { once: true }); }
  }

  function npProbeEnvironment() {
    var env = {
      userAgent: navigator.userAgent || "",
      pathname: window.location.pathname || "/",
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
      return env;
    } catch (e) { /* probes must never break the page */ }
    return null;
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
    return !!document.querySelector(".o_web_client, .o_action_manager, .o_main_content");
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
      var esc = function (x) { return String(x).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"); };
      var html = '<div style="font-size:0.85rem;color:#5b6472;margin-bottom:0.8rem;">الخادم المستهدف: <span dir="ltr" style="font-weight:600;">' + esc(origin) + '</span></div>';
      if (isTranslations) {
        html += '<div style="background:#fdf3e3;border:1px solid #f0d9a8;border-radius:12px;padding:0.7rem 0.9rem;font-size:0.85rem;color:#6b5410;margin-bottom:0.8rem;">خطأ في ملف الترجمة من خادم Odoo. حدّث إضافة native_print على الخادم إلى آخر إصدار ثم أعد تشغيل Odoo.</div>';
      }
      return html;
    }
    function errorRows() {
      var errs = [];
      try { errs = window.__NP_ERRORS__ || []; } catch (e) {}
      if (!errs.length) return "";
      var esc = function (x) { return String(x).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"); };
      var html = '<div style="margin-top:0.8rem;"><div style="font-size:0.85rem;font-weight:700;color:#b3261e;margin-bottom:0.3rem;">أخطاء الصفحة:</div>';
      for (var i = 0; i < Math.min(errs.length, 3); i++) {
        html += '<div style="font-size:0.78rem;color:#5b6472;word-break:break-word;padding:0.15rem 0;">' + esc(errs[i]) + '</div>';
      }
      return html + "</div>";
    }
    var btnBase = 'border:0;border-radius:999px;padding:0.65rem 1.2rem;font-size:0.9rem;cursor:pointer;background:#eef1f6;color:#101219;';
    el.innerHTML = '<div style="position:relative;max-width:460px;width:92vw;padding:1.4rem;background:#ffffff;border:1px solid #e3e8f0;border-radius:20px;box-shadow:0 12px 40px rgba(16,18,25,0.16);">' +
      '<button id="np-boot-dismiss" title="إخفاء" style="position:absolute;top:10px;inset-inline-end:12px;border:0;background:transparent;color:#8a93a5;font-size:1.05rem;line-height:1;cursor:pointer;padding:4px 6px;">&times;</button>' +
      '<div style="font-size:1.15rem;font-weight:800;color:#101219;margin-bottom:0.4rem;">واجهة Odoo لم تكتمل</div>' +
      '<div style="color:#5b6472;font-size:0.9rem;margin-bottom:1rem;">تعذّر بدء تطبيق Odoo بعد تحميل الصفحة. نتائج فحص بيئة العرض:</div>' +
      serverHint() +
      facts() +
      errorRows() +
      '<div style="display:flex;flex-wrap:wrap;gap:0.5rem;margin-top:1.2rem;">' +
      '<button id="np-boot-reload" style="' + btnBase + 'background:#0953DE;color:#fff;">إعادة التحميل</button>' +
      '<button id="np-boot-clear" style="' + btnBase + '">مسح بيانات التصفح</button>' +
      '<button id="np-boot-cc" style="' + btnBase + '">مركز التحكم</button>' +
      '</div>' +
      '<div style="color:#8a93a5;font-size:0.78rem;margin-top:0.9rem;">إذا لم تنجح إعادة التحميل، امسح بيانات التصفح من مركز التحكم (يُسجّل خروجك ويمسح ذاكرة التخزين المؤقت للتطبيق فقط).</div></div>';
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
      __npFacade.openControlCenter().catch(function () {});
    });
    var cc = document.getElementById("np-boot-cc");
    if (cc) cc.addEventListener("click", function () {
      __npFacade.openControlCenter().catch(function () {});
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
    var maxTicks = delays.failAfter + 300;
    function stopWatchdog() {
      window.clearInterval(timer);
      if (window.__NP_WATCHDOG_TIMER__ === timer) {
        window.__NP_WATCHDOG_TIMER__ = null;
      }
      window.__NP_WATCHDOG_ARMED__ = false;
    }
    var timer = window.setInterval(function () {
      ticks += 1;
      // The same document can soft-redirect to the login page
      // (session expiry): that page is healthy, never flag it.
      if (!npIsBackendPage()) {
        stopWatchdog();
        npBootOverlay("hide", env);
        return;
      }
      if (npBooted()) {
        if (!shellAt) shellAt = ticks;
        if (npContentReady()) {
          // Re-armable: later SPA route changes re-run the check
          // via the pushState/popstate hooks below.
          stopWatchdog();
          npBootOverlay("hide", env);
          return;
        }
        // Shell up, content still empty after a further grace
        // window: the white-content failure mode.
        if (!cardShown && ticks - shellAt >= 20) {
          cardShown = true;
          npBootOverlay("problem", npReportEnvironment() || npProbeEnvironment());
        }
        if (cardShown && ticks >= maxTicks) {
          stopWatchdog();
        }
        return;
      }
      // Show the diagnostic card once, then KEEP polling: a late boot
      // removes it automatically. There is deliberately no "loading"
      // cover any more — a slow boot is not a failed boot.
      if (!cardShown && ticks === delays.failAfter) { cardShown = true; npBootOverlay("problem", npReportEnvironment() || npProbeEnvironment()); }
      if (cardShown && ticks >= maxTicks) {
        stopWatchdog();
      }
    }, delays.tickMs);
    window.__NP_WATCHDOG_TIMER__ = timer;
  }
  if (__npIsTop && !npIsLocalPage()) {
    try {
      // Every Odoo report is intercepted by Native Print. Never expose
      // the browser's print dialog from an embedded remote page: it
      // bypasses printer selection, job logging and the native queue.
      window.print = function () {};
    } catch (e) { /* ignore */ }
    // Arm immediately at init-script evaluation: a page whose
    // DOMContentLoaded stalls (a hung render-blocking asset) still
    // gets its watchdog and environment report. Only the HTTP strip
    // needs document.body, and it waits for it itself.
    npReportEnvironment();
    npBootWatchdog();
    npHttpWarning();
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
  window.addEventListener("pagehide", function () {
    if (window.__NP_WATCHDOG_TIMER__) {
      window.clearInterval(window.__NP_WATCHDOG_TIMER__);
      window.__NP_WATCHDOG_TIMER__ = null;
      window.__NP_WATCHDOG_ARMED__ = false;
    }
    if (window.__NP_ERROR_TIMER__) {
      window.clearTimeout(window.__NP_ERROR_TIMER__);
      window.__NP_ERROR_TIMER__ = null;
    }
  });
  window.addEventListener("keydown", function (ev) {
    if (!__npIsTop) return;
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
      if (npIsLocalPage()) {
        __npFacade.openOdooOrigin().catch(function () {});
      } else {
        __npFacade.openControlCenter().catch(function () {});
      }
    }
  });
})();
"#;

/// Height of the tab bar strip at the top of the window (logical
/// pixels). Content webviews live below it.
pub const TABBAR_HEIGHT: f64 = 46.0;

fn bridge_js() -> &'static str {
    static SCRIPT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SCRIPT
        .get_or_init(|| BRIDGE_JS.replace("__APP_VERSION__", env!("CARGO_PKG_VERSION")))
        .as_str()
}

/// Fit the tab bar and the ACTIVE content webview to the window's
/// current client size. Called at startup and on every resize.
pub fn layout_webviews_with_size(app: &tauri::AppHandle, width: f64, height: f64) {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return;
    }
    if let Some(tabbar) = app.get_webview("tabbar") {
        let _ = tabbar.set_position(tauri::LogicalPosition::new(0.0, 0.0));
        let _ = tabbar.set_size(tauri::LogicalSize::new(width, TABBAR_HEIGHT));
    }
    let active = {
        let state = app.state::<AppState>();
        let tabs = commands::read_lock(&state.tabs);
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

pub fn layout_webviews(app: &tauri::AppHandle) {
    let Some(window) = app.get_window("main") else {
        return;
    };
    let Ok(size) = window.inner_size() else {
        return;
    };
    if size.width == 0 || size.height == 0 {
        return;
    }
    let scale = window
        .scale_factor()
        .ok()
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(1.0);
    layout_webviews_with_size(app, size.width as f64 / scale, size.height as f64 / scale);
}

/// Make `label` the visible tab: hide every other content webview,
/// show this one, refit it to the window.
pub fn activate_tab(app: &tauri::AppHandle, label: &str) {
    let labels = {
        let state = app.state::<AppState>();
        let mut tabs = commands::write_lock(&state.tabs);
        if !tabs.order.iter().any(|l| l == label) {
            return;
        }
        tabs.active = Some(label.to_string());
        tabs.order.clone()
    };
    layout_webviews(app);
    for other in &labels {
        if let Some(wv) = app.get_webview(other) {
            if other == label {
                let _ = wv.show();
            } else {
                let _ = wv.hide();
            }
        }
    }
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
    let open_label = label.to_string();
    let load_handle = app.clone();
    let load_label = label.to_string();
    let builder = tauri::webview::WebviewBuilder::new(label, url)
        .data_directory(data_dir)
        .disable_drag_drop_handler()
        .initialization_script(bridge_js())
        .on_navigation(move |u| {
            if !u.username().is_empty() || u.password().is_some() {
                return false;
            }
            if u.as_str() == "about:blank" || is_local_app_url(u) {
                return true;
            }
            let state = nav_handle.state::<AppState>();
            let verdict = {
                let allowed = commands::read_lock(&state.configured_origin);
                match allowed.as_deref() {
                    Some(cfg_origin) => {
                        let o = u.origin().ascii_serialization();
                        origin_is_allowed(cfg_origin, &o)
                    }
                    None => false,
                }
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
                let tabs = commands::read_lock(&state.tabs);
                let is_pos_tab = tabs.is_pos_tab(&nav_label);
                let current_is_local = tabs
                    .url_of(&nav_label)
                    .and_then(|s| tauri::Url::parse(s).ok())
                    .map(|cu| is_local_app_url(&cu))
                    .unwrap_or(false);
                drop(tabs);
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
            if !u.username().is_empty() || u.password().is_some() {
                return tauri::webview::NewWindowResponse::Deny;
            }
            let state = open_handle.state::<AppState>();
            let opener_is_local = commands::read_lock(&state.tabs)
                .url_of(&open_label)
                .and_then(|s| tauri::Url::parse(s).ok())
                .map(|cu| is_local_app_url(&cu))
                .unwrap_or(false);
            let on_origin = (opener_is_local && is_local_app_url(&u))
                || commands::read_lock(&state.configured_origin)
                    .as_deref()
                    .map(|cfg| origin_is_allowed(cfg, &u.origin().ascii_serialization()))
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
        .on_page_load(move |_webview, payload| match payload.event() {
            tauri::webview::PageLoadEvent::Started | tauri::webview::PageLoadEvent::Finished => {
                let st = load_handle.state::<AppState>();
                let mut tabs = commands::write_lock(&st.tabs);
                tabs.set_url(&load_label, payload.url().as_str());
            }
        });
    let (width, height) = {
        let scale = window
            .scale_factor()
            .ok()
            .filter(|s| s.is_finite() && *s > 0.0)
            .unwrap_or(1.0);
        match window.inner_size() {
            Ok(size) if size.width > 0 && size.height > 0 => {
                (size.width as f64 / scale, size.height as f64 / scale)
            }
            _ => (1280.0, 820.0),
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

#[cfg(windows)]
fn acquire_single_instance_guard() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            attrs: *mut std::ffi::c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> *mut std::ffi::c_void;
        fn GetLastError() -> u32;
    }
    const ERROR_ALREADY_EXISTS: u32 = 183;
    let wide_name: Vec<u16> = "Local\\YaseirPrintSingleInstance\0"
        .encode_utf16()
        .collect();
    let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 0, wide_name.as_ptr()) };
    if handle.is_null() {
        return true;
    }
    unsafe { GetLastError() != ERROR_ALREADY_EXISTS }
}

#[cfg(not(windows))]
fn acquire_single_instance_guard() -> bool {
    true
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if !acquire_single_instance_guard() {
        eprintln!("Yaseir Print: another instance is already running; exiting.");
        return;
    }
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
            let printer_settings =
                Arc::new(RwLock::new(config::load_printer_settings(&config_dir)));
            let worker_wake = Arc::new((Mutex::new(true), Condvar::new()));
            let worker_stop = Arc::new(AtomicBool::new(false));
            spawn_worker_loop(
                db_path.clone(),
                Arc::clone(&printer_settings),
                Arc::clone(&worker_wake),
                Arc::clone(&worker_stop),
            );
            // Warm up PDFium and printer discovery in the background so the
            // first user print job does not pay DLL load or spooler cold start.
            let _ = std::thread::Builder::new()
                .name("native-print-warmup".into())
                .spawn(|| {
                    let _ = commands::pdfium_ready();
                    let _ = print_win::discover_printers();
                });

            let initial_origin = resolved.odoo_origin.clone();
            app.manage(AppState {
                queue: queue_arc,
                configured_origin: RwLock::new(initial_origin.clone()),
                config_dir,
                environment,
                tabs: RwLock::new(TabsState::default()),
                started_at: std::time::Instant::now(),
                locked_by_policy: resolved.locked_by_policy,
                auto_reprint: RwLock::new(resolved.auto_reprint),
                worker_wake,
                worker_stop,
                printer_settings,
            });

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

            // Single window hosting a local tab bar (tabbar.html) plus
            // one child webview per tab. Webviews are created from setup
            // or async commands only to avoid Windows deadlocks.
            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("Yaseir Print")
                .inner_size(1280.0, 820.0)
                .min_inner_size(960.0, 640.0)
                .build()?;
            let _tabbar = window.add_child(
                tauri::webview::WebviewBuilder::new(
                    "tabbar",
                    WebviewUrl::App("tabbar.html".into()),
                )
                .disable_drag_drop_handler()
                .on_navigation(|u| is_local_app_url(u))
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny),
                tauri::LogicalPosition::new(0.0, 0.0),
                tauri::LogicalSize::new(1280.0, TABBAR_HEIGHT),
            )?;

            // The first tab: the boot page (origin configured) or
            // the Control Center (nothing configured yet).
            {
                let state = app.state::<AppState>();
                let mut tabs = commands::write_lock(&state.tabs);
                tabs.next_id = 1;
                let first_file = match initial_origin.as_deref() {
                    Some(_) => "boot.html",
                    None => "index.html",
                };
                let first_url = commands::local_app_url(first_file)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|_| format!("tauri://localhost/{first_file}"));
                tabs.register("tab-1", &first_url);
                tabs.active = Some("tab-1".to_string());
            }
            create_tab_webview(app.handle(), "tab-1", start_url)?;

            // Keep the tab bar and the active tab fitted to the
            // window on every resize.
            let resize_handle = app.handle().clone();
            window.on_window_event(move |event| match event {
                tauri::WindowEvent::ScaleFactorChanged {
                    scale_factor,
                    new_inner_size,
                    ..
                } if scale_factor.is_finite() && *scale_factor > 0.0 => {
                    layout_webviews_with_size(
                        &resize_handle,
                        new_inner_size.width as f64 / *scale_factor,
                        new_inner_size.height as f64 / *scale_factor,
                    );
                }
                tauri::WindowEvent::Resized(_) => {
                    layout_webviews(&resize_handle);
                }
                _ => {}
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
        .build(tauri::generate_context!())
        .expect("error while building Yaseir Print")
        .run(|app_handle, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    state.worker_stop.store(true, Ordering::Release);
                    state.notify_worker();
                }
            }
        });
}

(function () {
  if (window.__NP_BRIDGE_INSTALLED__) return;
  window.__NP_BRIDGE_INSTALLED__ = true;

  // Subframes get no facade, no listeners, no watchdog, and no environment IPC (ADR-0005).
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

  var __npNextId = 1;
  var __npPending = Object.create(null);

  function attachPortListener() {
    var port = window.__NP_ANDROID_PORT__;
    if (!port || port.__npBound) return;
    port.__npBound = true;
    port.onmessage = function (ev) {
      try {
        var msg = typeof ev.data === "string" ? JSON.parse(ev.data) : ev.data;
        if (!msg || !msg.id) return;
        var entry = __npPending[msg.id];
        if (!entry) return;
        delete __npPending[msg.id];
        if (msg.ok) {
          entry.resolve(msg.result);
        } else {
          var err = new Error(msg.error || "native_print_error");
          if (msg.code) err.code = msg.code;
          entry.reject(err);
        }
      } catch (e) { /* ignore malformed reply */ }
    };
  }
  attachPortListener();

  function invoke(cmd, args) {
    attachPortListener();
    var port = window.__NP_ANDROID_PORT__;
    if (!port || typeof port.postMessage !== "function") {
      return Promise.reject(new Error("android_bridge_unavailable"));
    }
    var id = "req_" + (__npNextId++);
    return new Promise(function (resolve, reject) {
      __npPending[id] = { resolve: resolve, reject: reject };
      try {
        port.postMessage(JSON.stringify({ id: id, cmd: cmd, args: args || {} }));
      } catch (e) {
        delete __npPending[id];
        reject(e);
      }
    });
  }

  // Page error tap (mirrors desktop BRIDGE_JS)
  window.__NP_ERRORS__ = [];
  function npPushError(kind, message) {
    try {
      var list = window.__NP_ERRORS__;
      if (list.length >= 10) return;
      var text = String(kind || "error") + ": " + String(message == null ? "" : message).slice(0, 300);
      if (list.indexOf(text) !== -1) return;
      list.push(text);
      if (!window.__NP_ERROR_TIMER__) {
        window.__NP_ERROR_TIMER__ = window.setTimeout(function () {
          window.__NP_ERROR_TIMER__ = null;
          npReportEnvironment();
        }, 300);
      }
      try {
        if (document.getElementById("np-boot-overlay")) {
          npBootOverlay("problem", npProbeEnvironment());
        }
      } catch (e) {}
    } catch (e) {}
  }

  window.addEventListener("error", function (ev) {
    var t = ev && ev.target;
    if (t && t !== window && (t.tagName === "SCRIPT" || t.tagName === "LINK")) {
      var assetUrl = String(t.src || t.href || "asset").split(/[?#]/)[0];
      npPushError("load", assetUrl + " failed to load");
    } else if (ev && ev.message) {
      npPushError("error", ev.message);
    }
  }, true);

  window.addEventListener("unhandledrejection", function (ev) {
    var r = ev && ev.reason;
    npPushError("promise", (r && (r.message || r)) || "unhandled rejection");
  });

  // Exact v1 facade (13 remote commands; admin actions are strictly native Kotlin UI only)
  var __npFacade = {
    version: "1.5.7",
    printPdf: function (args) {
      var payload = Object.assign({}, args, { bytes: normalizeBytes(args && args.bytes) });
      return invoke("print_pdf", { req: payload });
    },
    printText: function (args) {
      return invoke("print_text", { req: args });
    },
    printImage: function (args) {
      var payload = Object.assign({}, args, { bytes: normalizeBytes(args && args.bytes) });
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
    openControlCenter: function () {
      return invoke("open_control_center", {});
    },
    openSettings: function () {
      return invoke("open_settings", {});
    },
    openTab: function (url) {
      return invoke("tab_open", { url: String(url) });
    },
    reportEnvironment: function (report) {
      return invoke("report_environment", { report: report });
    }
  };

  try { Object.freeze(__npFacade); } catch (e) {}
  try {
    Object.defineProperty(window, "__NATIVE_PRINT__", {
      value: __npFacade,
      writable: false,
      configurable: false
    });
  } catch (e) {
    window.__NATIVE_PRINT__ = __npFacade;
  }

  // Suppress browser print dialog so all Odoo reports flow through Native Print
  try { window.print = function () {}; } catch (e) {}

  function npProbeEnvironment() {
    var env = {
      userAgent: navigator.userAgent || "",
      pathname: window.location.pathname || "/",
      secureContext: window.isSecureContext === true,
      online: navigator.onLine !== false
    };
    try { env.es2022 = typeof Array.prototype.at === "function" && typeof Object.hasOwn === "function"; } catch (e) { env.es2022 = false; }
    try { env.structuredClone = typeof structuredClone === "function"; } catch (e) { env.structuredClone = false; }
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
    } catch (e) { return null; }
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
    return !!document.querySelector(".o_web_client, .o_action_manager, .o_main_content");
  }

  function npContentReady() {
    try { if ((window.location.pathname || "").indexOf("/pos/") !== -1) return true; } catch (e) {}
    var am = document.querySelector(".o_action_manager");
    if (am && am.children.length > 0) return true;
    return !!document.querySelector(".o-mail-Discuss, .o_mail_discuss, .o_kanban_view, .o_list_view, .o_form_view, .o_pivot_view, .o_graph_view, .o_calendar_view");
  }

  function npBootOverlay(mode, env) {
    var existing = document.getElementById("np-boot-overlay");
    if (mode === "hide") { if (existing) existing.remove(); return; }
    if (window.__NP_CARD_DISMISSED__ || !document.body) return;
    var el = existing;
    if (!el) {
      el = document.createElement("div");
      el.id = "np-boot-overlay";
      el.setAttribute("style", "position:fixed;right:12px;bottom:12px;z-index:2147483647;font-family:system-ui,sans-serif;direction:rtl;");
      document.body.appendChild(el);
    }
    el.innerHTML = '<div style="max-width:360px;width:90vw;padding:1rem;background:#fff;border:1px solid #e7e4e4;border-radius:16px;box-shadow:0 10px 30px rgba(14,20,43,0.16);color:#0e142b;">' +
      '<div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:0.4rem;">' +
      '<strong style="font-size:1rem;">واجهة Odoo لم تكتمل</strong>' +
      '<button id="np-boot-dismiss" style="border:0;background:transparent;font-size:1.1rem;cursor:pointer;color:#555c72;">&times;</button></div>' +
      '<div style="font-size:0.85rem;color:#555c72;margin-bottom:0.8rem;">تعذّر بدء تطبيق Odoo بعد تحميل الصفحة.</div>' +
      '<div style="display:flex;gap:0.5rem;flex-wrap:wrap;">' +
      '<button id="np-boot-reload" style="border:0;border-radius:999px;padding:0.45rem 1rem;background:#2e67ff;color:#fff;font-size:0.85rem;cursor:pointer;">إعادة التحميل</button>' +
      '<button id="np-boot-cc" style="border:0;border-radius:999px;padding:0.45rem 1rem;background:#f3f4f6;color:#0e142b;font-size:0.85rem;cursor:pointer;">مركز التحكم</button>' +
      '</div></div>';
    var reload = document.getElementById("np-boot-reload");
    if (reload) reload.addEventListener("click", function () { window.location.reload(); });
    var dismiss = document.getElementById("np-boot-dismiss");
    if (dismiss) dismiss.addEventListener("click", function () { window.__NP_CARD_DISMISSED__ = true; el.remove(); });
    var cc = document.getElementById("np-boot-cc");
    if (cc) cc.addEventListener("click", function () { __npFacade.openControlCenter().catch(function () {}); });
  }

  function npBootWatchdog() {
    if (window.__NP_WATCHDOG_ARMED__ || !npIsBackendPage()) return;
    window.__NP_WATCHDOG_ARMED__ = true;
    var ticks = 0, shellAt = 0, cardShown = false;
    var timer = window.setInterval(function () {
      ticks += 1;
      if (!npIsBackendPage()) { window.clearInterval(timer); window.__NP_WATCHDOG_ARMED__ = false; npBootOverlay("hide", null); return; }
      if (npBooted()) {
        if (!shellAt) shellAt = ticks;
        if (npContentReady()) { window.clearInterval(timer); window.__NP_WATCHDOG_ARMED__ = false; npBootOverlay("hide", null); return; }
        if (!cardShown && ticks - shellAt >= 20) { cardShown = true; npBootOverlay("problem", npReportEnvironment() || npProbeEnvironment()); }
        return;
      }
      if (!cardShown && ticks === 30) { cardShown = true; npBootOverlay("problem", npReportEnvironment() || npProbeEnvironment()); }
      if (ticks >= 330) { window.clearInterval(timer); window.__NP_WATCHDOG_ARMED__ = false; }
    }, 1000);
    window.__NP_WATCHDOG_TIMER__ = timer;
  }

  npReportEnvironment();
  npBootWatchdog();
  try {
    var npPushState = window.history.pushState;
    window.history.pushState = function () {
      var r = npPushState.apply(this, arguments);
      window.setTimeout(npBootWatchdog, 0);
      return r;
    };
    window.addEventListener("popstate", function () { npBootWatchdog(); });
  } catch (e) {}
})();

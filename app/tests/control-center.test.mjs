import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const html = readFileSync(new URL("../dist/index.html", import.meta.url), "utf8");
const scripts = [...html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/gi)];
assert.equal(scripts.length, 1, "Control Center must have one embedded app script");
const script = scripts[0][1];

test("embedded Control Center JavaScript parses", () => {
  assert.doesNotThrow(() => new vm.Script(script));
});

test("no inline event handlers remain and all data-action bindings resolve", () => {
  assert.equal(html.match(/\sstyle\s*=\s*"/gi), null, "no inline style= attributes allowed in index.html (Tauri v2 CSP auto-hashing drops unsafe-inline)");
  const inlineHandlers = [...html.matchAll(/\bon(?:click|change|input|submit)=["']([^"']+)["']/gi)];
  assert.equal(inlineHandlers.length, 0, "inline event handlers must be removed for strict CSP");
  const actions = [...html.matchAll(/data-action=["']([^"']+)["']/g)].map(m => m[1]);
  assert.ok(actions.length >= 25, "expected data-action bindings on controls");
  assert.match(script, /document\.addEventListener\("click"/);
  assert.match(script, /document\.addEventListener\("change"/);
  assert.match(script, /getElementById\("confirmOverlay"\)\.addEventListener\("click", confirmOverlayClicked\)/);
  for (const action of new Set(actions)) {
    assert.ok(script.includes(`action === "${action}"`), "unhandled data-action: " + action);
  }
});

test("static DOM lookups resolve and element IDs are unique", () => {
  // (?<![-\w]) keeps data-* attributes (e.g. data-job-id) out: only
  // real element id attributes count for uniqueness.
  const ids = [...html.matchAll(/(?<![-\w])id=["']([^"']+)["']/g)].map(m => m[1]);
  const set = new Set(ids);
  assert.equal(set.size, ids.length);
  const refs = [...script.matchAll(/getElementById\(["']([^"'$]+)["']\)/g)].map(m => m[1]);
  assert.deepEqual([...new Set(refs.filter(id => !set.has(id)))], []);
});

test("localized controls have Arabic and English translations", () => {
  const match = script.match(/const I18N = (\{[\s\S]*?\n  \});/);
  assert.ok(match);
  const i18n = new vm.Script("(" + match[1] + ")").runInNewContext({});
  assert.deepEqual(Object.keys(i18n.ar).sort(), Object.keys(i18n.en).sort(), "I18N AR/EN key mismatch");
  for (const key of html.matchAll(/data-i18n=["']([^"']+)["']/g)) {
    assert.ok(i18n.ar[key[1]], "missing Arabic translation: " + key[1]);
    assert.ok(i18n.en[key[1]], "missing English translation: " + key[1]);
  }
});

function extract(name, marker) {
  const start = script.indexOf("function " + name + "(");
  const end = script.indexOf(marker, start);
  assert.ok(start >= 0 && end > start, "function not found: " + name);
  return script.slice(start, end).trim();
}
const validOdoo = new vm.Script("(" + extract("isAllowedOrigin", "  function isAllowedPrinterUri(") + ")").runInNewContext({ URL });
const validPrinter = new vm.Script("(" + extract("isAllowedPrinterUri", "  // Live field validation:") + ")").runInNewContext({ URL });

test("Odoo origin accepts HTTPS anywhere and HTTP on any IP", () => {
  for (const value of [
    "https://odoo.company.local:8069", "https://odoo.example.com",
    "https://192.168.1.20:8069", "https://8.8.8.8:8069",
    "http://192.168.1.20:8069", "http://10.0.0.5:8069",
    "http://172.16.4.9:8069", "http://127.0.0.1:8069",
    "http://8.8.8.8:8069", "http://203.0.113.10:8070",
    "http://[::1]:8069", "http://[fd00::25]:8069", "http://[fe80::1]:8069",
    "http://[2001:4860:4860::8888]:8069",
    "http://localhost:8069"
  ]) assert.equal(validOdoo(value), true, value);
});

test("Odoo origin rejects HTTP hostnames, bad IPs and URL tricks", () => {
  for (const value of [
    "http://odoo.company.local:8069",
    "http://evil.localhost:8069",
    "http://999.999.999.999:8069",
    "http://192.168.1.20:0", "http://192.168.1.20:65536",
    "http://user@192.168.1.20:8069", "https://user@odoo.example.com:8069",
    "http://192.168.1.20:8069/web", "https://odoo.example.com/web",
    "http://192.168.1.20:8069/?debug=1", "http://192.168.1.20:8069#x",
    "ftp://192.168.1.20:8069"
  ]) assert.equal(validOdoo(value), false, value);
});

test("printer URI accepts IP-only TCP, RAW and IPP", () => {
  for (const value of [
    "tcp://192.168.1.200:9100", "raw://192.168.1.200:9100",
    "tcp://192.168.0.10:9100", "tcp://10.0.0.5:9100", "tcp://172.16.0.1:9100",
    "ipp://192.168.0.50:631/ipp/print",
    "ipp://192.168.1.200:631/ipp/print", "ipp://[::1]:631/ipp/print"
  ]) assert.equal(validPrinter(value), true, value);
});

test("printer URI rejects domains, other schemes and bad ports", () => {
  for (const value of [
    "tcp://printer.local:9100", "https://192.168.1.200:9100",
    "ftp://192.168.1.200:21", "tcp://999.999.999.999:9100",
    "tcp://192.168.1.200:0", "tcp://192.168.1.200:65536",
    "tcp://192.168.00.1:9100", "tcp://0001:9100",
    "tcp://192.168.1.200:9100/print"
  ]) assert.equal(validPrinter(value), false, value);
});

test("demo fallback and local Windows paths are removed", () => {
  assert.equal(html.includes("FALLBACK_DEMO"), false);
  assert.equal(html.includes("Preview:"), false);
  assert.equal(html.includes("txtConfigPath"), false);
  assert.equal(html.includes("txtPdfiumPath"), false);
  assert.match(script, /async function saveAndOpenOdoo\(\)/);
  assert.match(script, /document\.execCommand\("copy"\)/);
  assert.match(script, /window\.setTimeout\(\(\) =>/);
});

test("printer card CSS classes match rendered DOM and diagnostics output is preserved on language switch", () => {
  for (const cls of [".printer-card-top", ".printer-identity", ".printer-type-icon", ".printer-meta-list", ".printer-meta-item", ".badge .dot"]) {
    assert.ok(html.includes(cls), "missing CSS rule for " + cls);
  }
  assert.match(script, /el\.id === "diagnosticsOutput" && diagnosticsGenerated/);
});

test("secondary desktop pages parse, resolve IDs, translate all keys, and match Rust IPC commands", () => {
  const libRs = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  const handlerBlock = libRs.match(/tauri::generate_handler!\[([\s\S]*?)\]/)?.[1] || "";
  const registered = new Set([...handlerBlock.matchAll(/commands::(\w+)/g)].map(m => m[1]));
  assert.ok(registered.size >= 25, "expected registered Tauri commands in lib.rs");

  for (const file of ["boot.html", "tabbar.html", "settings.html"]) {
    const pageHtml = readFileSync(new URL("../dist/" + file, import.meta.url), "utf8");
    const pageScripts = [...pageHtml.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/gi)];
    assert.equal(pageScripts.length, 1, file + " must have one embedded script");
    const pageScript = pageScripts[0][1];
    assert.doesNotThrow(() => new vm.Script(pageScript), file + " JS syntax");

    const ids = [...pageHtml.matchAll(/(?<![-\w])id=["']([^"']+)["']/g)].map(m => m[1]);
    const idSet = new Set(ids);
    assert.equal(idSet.size, ids.length, file + " duplicate IDs");
    const refs = [...pageScript.matchAll(/getElementById\(["']([^"'$]+)["']\)/g)].map(m => m[1]);
    assert.deepEqual([...new Set(refs.filter(id => !idSet.has(id)))], [], file + " missing DOM IDs");

    const strMatch = pageScript.match(/var STR = (\{[\s\S]*?\n  \});/);
    if (strMatch) {
      const str = new vm.Script("(" + strMatch[1] + ")").runInNewContext({});
      assert.deepEqual(Object.keys(str.ar).sort(), Object.keys(str.en).sort(), file + " AR/EN key mismatch");
      for (const key of pageHtml.matchAll(/data-i18n=["']([^"']+)["']/g)) {
        assert.ok(str.ar[key[1]], file + " missing AR translation: " + key[1]);
        assert.ok(str.en[key[1]], file + " missing EN translation: " + key[1]);
      }
    }

    for (const inv of pageScript.matchAll(/\binvoke\(\s*["']([^"']+)["']/g)) {
      assert.ok(registered.has(inv[1]), file + " invokes unregistered Tauri command: " + inv[1]);
    }
  }
  const bootHtml = readFileSync(new URL("../dist/boot.html", import.meta.url), "utf8");
  assert.match(bootHtml, /function isHttpOrigin\(/, "boot.html must validate http/https origin before redirect");
  const admx = readFileSync(new URL("../deploy/admx/OdooNativePrint.admx", import.meta.url), "utf8");
  const odooOriginPolicy = admx.match(/<policy name="OdooOrigin"[^>]*>/)?.[0] || "";
  assert.equal(odooOriginPolicy.includes("valueName="), false, "ADMX OdooOrigin <policy> must not duplicate valueName of <text>");
});

const escapeHtml = new vm.Script("(" + extract("escapeHtml", "  function formatBytes(") + ")").runInNewContext({});

test("escapeHtml neutralizes HTML special characters", () => {
  assert.equal(escapeHtml(`<script>alert("x" & 'y')</script>`), "&lt;script&gt;alert(&quot;x&quot; &amp; &#39;y&#39;)&lt;/script&gt;");
  assert.equal(escapeHtml(null), "");
  assert.equal(escapeHtml(undefined), "");
});

test("Control Center bridge calls align with lib.rs BRIDGE_JS and registered commands", async () => {
  const { extractBridge } = await import("./jsdom/extract-bridge.mjs");
  const bridgeJs = extractBridge();
  assert.doesNotThrow(() => new vm.Script(bridgeJs), "extracted BRIDGE_JS must parse");
  const libRs = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  const handlerBlock = libRs.match(/tauri::generate_handler!\[([\s\S]*?)\]/)?.[1] || "";
  const registered = new Set([...handlerBlock.matchAll(/commands::(\w+)/g)].map(m => m[1]));
  for (const inv of bridgeJs.matchAll(/\binvoke\(\s*["']([^"']+)["']/g)) {
    assert.ok(registered.has(inv[1]), "BRIDGE_JS invokes unregistered command: " + inv[1]);
  }
  const facadeMethods = new Set([...bridgeJs.matchAll(/\b(\w+):\s*function\s*\(/g)].map(m => m[1]));
  for (const call of script.matchAll(/\b(?:api|b)\.(\w+)\s*\(/g)) {
    if (["has", "get", "set", "delete", "add", "remove", "toggle", "includes", "forEach", "map", "filter", "find", "slice", "join", "push", "trim", "replace", "toLowerCase", "toUpperCase", "startsWith", "endsWith", "test", "match", "split", "setAttribute", "getAttribute", "closest", "querySelector", "querySelectorAll", "addEventListener", "focus", "click"].includes(call[1])) continue;
    assert.ok(facadeMethods.has(call[1]), "index.html calls missing bridge method: " + call[1]);
  }
});

test("background polling pauses when document.hidden is true and refreshes on visibilitychange", () => {
  const bootIdx = script.lastIndexOf("refreshAll();");
  assert.ok(bootIdx >= 0, "boot refreshAll() call not found");
  const tail = script.slice(bootIdx);
  let intervalFn = null;
  let intervalMs = null;
  const listeners = new Map();
  const calls = [];
  const doc = {
    hidden: false,
    addEventListener(type, fn) { listeners.set(type, fn); },
  };
  new vm.Script(tail).runInNewContext({
    document: doc,
    refreshAll(opts) { calls.push(opts || null); },
    setInterval(fn, ms) { intervalFn = fn; intervalMs = ms; return 1; },
  });
  assert.equal(intervalMs, 5000);
  assert.equal(typeof intervalFn, "function");
  assert.equal(typeof listeners.get("visibilitychange"), "function", "missing visibilitychange listener");
  assert.equal(calls.length, 1, "expected initial refreshAll()");

  doc.hidden = true;
  intervalFn();
  assert.equal(calls.length, 1, "poll must not run while document.hidden is true");
  listeners.get("visibilitychange")();
  assert.equal(calls.length, 1, "visibilitychange while hidden must not refresh");

  doc.hidden = false;
  listeners.get("visibilitychange")();
  assert.equal(calls.length, 2, "becoming visible must trigger immediate silent refresh");
  assert.equal(calls[1]?.silent, true);

  intervalFn();
  assert.equal(calls.length, 3, "poll must resume while visible");
  assert.equal(calls[2]?.silent, true);
});

test("settings has no dead .example links and all desktop pages declare button types and toast live region", () => {
  const settingsHtml = readFileSync(new URL("../dist/settings.html", import.meta.url), "utf8");
  assert.doesNotMatch(settingsHtml, /<a\b[^>]*\.example/i, "settings.html must not link to dead .example URLs");
  assert.doesNotMatch(settingsHtml, /\.example\b/i, "settings.html must not contain .example placeholder strings");
  assert.doesNotMatch(settingsHtml, /0000\s+0000/, "settings.html must not contain placeholder phone numbers");
  assert.doesNotMatch(settingsHtml, /\.row\s+\.v\s+a\b/, "dead anchor CSS in settings.html must be removed");
  assert.match(html, /id="toastContainer"[^>]*aria-live="polite"/, "toastContainer must declare aria-live=polite");
  for (const file of ["index.html", "settings.html", "boot.html", "tabbar.html"]) {
    const pageHtml = readFileSync(new URL("../dist/" + file, import.meta.url), "utf8");
    const badButtons = [...pageHtml.matchAll(/<button\b[^>]*>/gi)]
      .map(m => m[0])
      .filter(tag => !/\btype=["'](?:button|submit|reset)["']/i.test(tag));
    assert.deepEqual(badButtons, [], file + " has <button> elements without explicit type=");
  }
});

test("theme.css Light and Dark tokens meet WCAG 2.2 AA contrast thresholds", () => {
  const css = readFileSync(new URL("../dist/theme.css", import.meta.url), "utf8");
  const parseBlock = (re) => {
    const m = css.match(re);
    assert.ok(m, "missing CSS token block: " + re);
    const vars = {};
    for (const [, k, v] of m[1].matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/gi)) {
      vars[k] = v.trim();
    }
    return vars;
  };
  const parseColor = (val, baseRgb) => {
    const hex = val.match(/^#([0-9a-f]{6})$/i);
    if (hex) {
      const n = parseInt(hex[1], 16);
      return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
    }
    const rgba = val.match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)(?:\s*,\s*([\d.]+))?\s*\)$/i);
    if (rgba) {
      const r = +rgba[1], g = +rgba[2], b = +rgba[3], a = rgba[4] === undefined ? 1 : +rgba[4];
      if (a >= 1 || !baseRgb) return [r, g, b];
      return [
        Math.round(r * a + baseRgb[0] * (1 - a)),
        Math.round(g * a + baseRgb[1] * (1 - a)),
        Math.round(b * a + baseRgb[2] * (1 - a)),
      ];
    }
    assert.fail("unsupported color format: " + val);
  };
  const luminance = ([r, g, b]) => {
    const lin = (c) => {
      const s = c / 255;
      return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
  };
  const ratio = (fg, bg) => {
    const l1 = luminance(fg), l2 = luminance(bg);
    return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
  };

  const themes = [
    ["light", parseBlock(/:root\s*\{([\s\S]*?)\n\}/)],
    ["dark", parseBlock(/:root\[data-theme="dark"\]\s*\{([\s\S]*?)\n\}/)],
  ];

  for (const [name, vars] of themes) {
    const surface = parseColor(vars["--bg-surface"]);
    const canvas = parseColor(vars["--bg-canvas"]);
    const subtle = parseColor(vars["--bg-subtle"]);

    // Core text tokens >= 4.5:1 on surface, canvas, and subtle
    for (const token of ["--text-primary", "--text-secondary", "--text-muted", "--brand-text"]) {
      const fg = parseColor(vars[token]);
      for (const [bgName, bg] of [["--bg-surface", surface], ["--bg-canvas", canvas], ["--bg-subtle", subtle]]) {
        const cr = ratio(fg, bg);
        assert.ok(cr >= 4.5, `${name} ${token} on ${bgName} contrast ${cr.toFixed(2)}:1 < 4.5:1`);
      }
    }

    // Brand & danger button text >= 4.5:1 on fill & hover
    for (const [fgToken, bgTokens] of [["--brand-fg", ["--brand", "--brand-hover"]], ["--danger-fg", ["--danger", "--danger-hover"]]]) {
      for (const bgToken of bgTokens) {
        const cr = ratio(parseColor(vars[fgToken]), parseColor(vars[bgToken]));
        assert.ok(cr >= 4.5, `${name} ${fgToken} on ${bgToken} contrast ${cr.toFixed(2)}:1 < 4.5:1`);
      }
    }

    // Semantic status fg >= 4.5:1 on surface and on its own status bg
    for (const kind of ["ok", "warn", "err", "info"]) {
      const fg = parseColor(vars[`--${kind}-fg`]);
      const crSurface = ratio(fg, surface);
      assert.ok(crSurface >= 4.5, `${name} --${kind}-fg on --bg-surface contrast ${crSurface.toFixed(2)}:1 < 4.5:1`);
      const statusBg = parseColor(vars[`--${kind}-bg`], surface);
      const crStatus = ratio(fg, statusBg);
      assert.ok(crStatus >= 4.5, `${name} --${kind}-fg on --${kind}-bg contrast ${crStatus.toFixed(2)}:1 < 4.5:1`);
    }

    // Non-text UI control border >= 3.0:1 on surface (WCAG 1.4.11)
    const crBorder = ratio(parseColor(vars["--border-strong"]), surface);
    assert.ok(crBorder >= 3.0, `${name} --border-strong on --bg-surface contrast ${crBorder.toFixed(2)}:1 < 3.0:1`);
  }
});

test("all desktop pages enforce strict CSP (no inline handlers) and ARIA landmarks/SVG hygiene", () => {
  for (const file of ["index.html", "boot.html", "tabbar.html", "settings.html"]) {
    const pageHtml = readFileSync(new URL("../dist/" + file, import.meta.url), "utf8");
    const markupOnly = pageHtml.slice(0, pageHtml.indexOf("<script"));
    const inlineHandlers = [...markupOnly.matchAll(/\bon[a-z]+\s*=\s*["'][^"']*["']/gi)];
    assert.deepEqual(inlineHandlers.map(m => m[0]), [], file + " must not contain inline event handlers");
    const unhiddenSvgs = [...markupOnly.matchAll(/<svg\b[^>]*>/gi)]
      .map(m => m[0])
      .filter(tag => !/\baria-hidden=["']true["']/i.test(tag));
    assert.deepEqual(unhiddenSvgs, [], file + " has decorative <svg> without aria-hidden=true");
  }
  const bootHtml = readFileSync(new URL("../dist/boot.html", import.meta.url), "utf8");
  assert.doesNotMatch(bootHtml, /<main\b[^>]*role=["']status["']/i, "boot.html <main> must not be overridden by role=status");
  const settingsHtml = readFileSync(new URL("../dist/settings.html", import.meta.url), "utf8");
  for (const m of settingsHtml.matchAll(/<div\b[^>]*role=["']group["'][^>]*>/gi)) {
    assert.match(m[0], /\baria-labelledby=["'][^"']+["']/, "settings.html role=group must have aria-labelledby");
  }
});

// Minimal DOM harness for executing real page scripts in node:vm without external deps
function createMiniDom(pageHtml, { lang = "ar", dir = "rtl" } = {}) {
  const docListeners = new Map();
  const winListeners = new Map();
  const byId = new Map();
  const allEls = [];

  class MiniClassList {
    constructor(el) { this.el = el; this.set = new Set(); }
    add(...c) { c.forEach(x => x && this.set.add(x)); }
    remove(...c) { c.forEach(x => this.set.delete(x)); }
    contains(c) { return this.set.has(c); }
    toggle(c, force) {
      if (force === undefined) { if (this.set.has(c)) { this.set.delete(c); return false; } this.set.add(c); return true; }
      if (force) this.set.add(c); else this.set.delete(c);
      return !!force;
    }
  }

  class MiniEl {
    constructor(tag, id = "") {
      this.tagName = String(tag || "DIV").toUpperCase();
      this.id = id;
      this.attrs = new Map();
      if (id) this.attrs.set("id", id);
      this.classList = new MiniClassList(this);
      this.dataset = {};
      this.style = {};
      this.children = [];
      this.parentNode = null;
      this.listeners = new Map();
      this._text = "";
      this._html = "";
      this.value = "";
      this.disabled = false;
      this.title = "";
      this.placeholder = "";
      this.draggable = false;
      this.firstChild = { nodeType: 3, nodeValue: "" };
      this.scrollIntoView = () => {};
    }
    get className() { return [...this.classList.set].join(" "); }
    set className(v) { this.classList.set = new Set(String(v || "").split(/\s+/).filter(Boolean)); }
    get textContent() { return this._text || this.children.map(c => c.textContent).join(""); }
    set textContent(v) { this._text = String(v == null ? "" : v); this.children = []; this._html = ""; }
    get innerHTML() { return this._html; }
    set innerHTML(v) { this._html = String(v == null ? "" : v); this.children = []; this._text = ""; }
    setAttribute(k, v) {
      this.attrs.set(k, String(v));
      if (k === "id") { this.id = String(v); byId.set(this.id, this); }
      if (k === "class") this.className = String(v);
      if (k.startsWith("data-")) {
        const camel = k.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase());
        this.dataset[camel] = String(v);
      }
    }
    getAttribute(k) { return this.attrs.has(k) ? this.attrs.get(k) : null; }
    appendChild(child) {
      if (child && typeof child === "object") {
        child.parentNode = this;
        this.children.push(child);
        if (child.id) byId.set(child.id, child);
      }
      return child;
    }
    prepend(child) {
      if (child && typeof child === "object") {
        child.parentNode = this;
        this.children.unshift(child);
      }
      return child;
    }
    remove() {
      if (!this.parentNode) return;
      const i = this.parentNode.children.indexOf(this);
      if (i >= 0) this.parentNode.children.splice(i, 1);
      this.parentNode = null;
    }
    contains(other) {
      let cur = other;
      while (cur) { if (cur === this) return true; cur = cur.parentNode; }
      return false;
    }
    addEventListener(type, fn) {
      if (!this.listeners.has(type)) this.listeners.set(type, []);
      this.listeners.get(type).push(fn);
    }
    dispatch(type, ev = {}) {
      const event = { target: this, currentTarget: this, preventDefault() {}, stopPropagation() {}, ...ev };
      for (const fn of this.listeners.get(type) || []) fn(event);
      return event;
    }
    focus() { document.activeElement = this; }
    select() {}
    getBoundingClientRect() { return { left: 0, width: 100, top: 0, height: 34 }; }
    querySelectorAll(sel) {
      const out = [];
      const walk = (n) => {
        for (const c of n.children) {
          if (matchesSel(c, sel)) out.push(c);
          walk(c);
        }
      };
      walk(this);
      return out;
    }
    querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
  }

  function matchesSel(el, sel) {
    if (!el || !el.classList) return false;
    if (sel === ".tab") return el.classList.contains("tab");
    if (sel === ".tab.active") return el.classList.contains("tab") && el.classList.contains("active");
    if (sel === ".tab.drop-hint") return el.classList.contains("tab") && el.classList.contains("drop-hint");
    if (sel === ".toast-close") return el.classList.contains("toast-close") || el._html.includes("toast-close");
    if (sel === ".nav-tab") return el.classList.contains("nav-tab");
    if (sel === ".tab-pane") return el.classList.contains("tab-pane");
    if (sel === ".filter-pill") return el.classList.contains("filter-pill");
    if (sel === ".main-container") return el.classList.contains("main-container");
    if (sel === ".card") return el.classList.contains("card");
    if (sel === "[data-i18n]") return el.attrs.has("data-i18n");
    if (sel === "#sideNav button") return el.parentNode && el.parentNode.id === "sideNav" && el.tagName === "BUTTON";
    if (sel === "section.page") return el.tagName === "SECTION" && el.classList.contains("page");
    if (sel.startsWith('.tab[data-label="')) {
      const lbl = sel.slice(17, -2);
      return el.classList.contains("tab") && el.getAttribute("data-label") === lbl;
    }
    return false;
  }

  // Parse static elements with id, class, data-i18n, data-page, data-tab, data-filter from markup
  const markup = pageHtml.slice(0, pageHtml.indexOf("<script"));
  const body = new MiniEl("BODY");
  const tagRe = /<(main|nav|section|div|button|input|select|pre|span|h1|p|footer|tbody)\b([^>]*)>/gi;
  for (const m of markup.matchAll(tagRe)) {
    const tag = m[1], attrStr = m[2];
    const el = new MiniEl(tag);
    for (const a of attrStr.matchAll(/([\w-]+)=["']([^"']*)["']/g)) {
      el.setAttribute(a[1], a[2]);
    }
    if (tag.toLowerCase() === "button" && el.attrs.has("data-page") && byId.has("sideNav")) {
      byId.get("sideNav").appendChild(el);
    } else if (tag.toLowerCase() === "button" && el.classList.contains("nav-tab") && byId.has("navTabs")) {
      byId.get("navTabs").appendChild(el);
    } else {
      body.appendChild(el);
    }
    allEls.push(el);
  }

  const storage = new Map();
  const timers = [];
  const intervals = [];
  const document = {
    hidden: false,
    title: "",
    activeElement: null,
    documentElement: {
      lang, dir,
      attrs: new Map([["lang", lang], ["dir", dir], ["data-theme", "light"]]),
      setAttribute(k, v) { this.attrs.set(k, String(v)); if (k === "lang") this.lang = String(v); if (k === "dir") this.dir = String(v); },
      getAttribute(k) { return this.attrs.get(k) || null; },
    },
    body,
    getElementById(id) { return byId.get(id) || null; },
    createElement(tag) { return new MiniEl(tag); },
    createTextNode(t) { return { nodeType: 3, textContent: String(t) }; },
    querySelectorAll(sel) {
      if (sel.startsWith('[id^="forms-box-"]')) return [...byId.values()].filter(e => e.id.startsWith("forms-box-"));
      if (sel.startsWith('[data-action="inspect-forms"]')) return [];
      if (sel.startsWith('[data-action="test-print"]')) return [];
      return allEls.filter(e => matchesSel(e, sel));
    },
    querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
    addEventListener(type, fn) {
      if (!docListeners.has(type)) docListeners.set(type, []);
      docListeners.get(type).push(fn);
    },
    dispatch(type, ev = {}) {
      for (const fn of docListeners.get(type) || []) fn(ev);
    },
  };

  const replacedUrls = [];
  const windowObj = {
    document,
    scrollY: 0,
    scrollTo() {},
    location: { replace(u) { replacedUrls.push(u); } },
    setInterval(fn, ms) { const id = intervals.length + 1; intervals.push({ id, fn, ms, cleared: false }); return id; },
    clearInterval(id) { const t = intervals.find(x => x.id === id); if (t) t.cleared = true; },
    setTimeout(fn, ms) { const id = timers.length + 1; timers.push({ id, fn, ms, cleared: false }); return id; },
    clearTimeout(id) { const t = timers.find(x => x.id === id); if (t) t.cleared = true; },
    addEventListener(type, fn) {
      if (!winListeners.has(type)) winListeners.set(type, []);
      winListeners.get(type).push(fn);
    },
    dispatch(type, ev = {}) {
      for (const fn of winListeners.get(type) || []) fn(ev);
    },
    localStorage: {
      getItem(k) { return storage.has(k) ? storage.get(k) : null; },
      setItem(k, v) { storage.set(k, String(v)); },
      removeItem(k) { storage.delete(k); },
    },
    URL,
    console: { error() {}, warn() {}, log() {} },
  };
  windowObj.window = windowObj;
  return { windowObj, document, storage, timers, intervals, replacedUrls, MiniEl };
}

test("Control Center flows: overview, XSS escaping, race-free refresh, filters, test print, inspectForms, diagnostics, and i18n/theme", async () => {
  const { windowObj, document, storage, MiniEl } = createMiniDom(html);
  let overviewCalls = 0;
  let resolveSlowOverview = null;
  let nextOverviewPayload = {
    appVersion: "1.5.7",
    printers: [
      { name: 'HP <img src=x onerror=alert(1)>', kind: "local", is_default: true, status: "ready", port: "USB001", driver: "HP" },
      { name: "Microsoft Print to PDF", kind: "virtual", is_default: false, status: "ready", port: "PORTPROMPT:", driver: "MS" },
    ],
    defaultPrinter: 'HP <img src=x onerror=alert(1)>',
    stateCounts: [["queued", 2], ["done", 5], ["failed", 1]],
    pdfiumReady: true,
    webviewVersion: "120.0",
    environment: { es2022: true, localStorage: true, cookies: true, webSocket: true, secureContext: true, errors: ["<script>xss</script>"] },
    odooOrigin: "https://odoo.example.com",
    lockedByPolicy: false,
    autoReprint: false,
    recentJobs: [
      { id: "j-1", report: "<b>Invoice</b>", printer: "HP", pages: 2, bytes: 2048, state: "failed", attempts: 2, error_code: "ERR_<x>" },
      { id: "j-2", report: "Receipt", printer: "HP", pages: 1, bytes: 512, state: "done", attempts: 1, error_code: "" },
    ],
  };
  const testPrints = [];
  windowObj.__NATIVE_PRINT__ = {
    async appOverview() {
      overviewCalls++;
      if (resolveSlowOverview) {
        const fn = resolveSlowOverview;
        resolveSlowOverview = null;
        return new Promise((res) => fn(() => res(JSON.parse(JSON.stringify(nextOverviewPayload)))));
      }
      return JSON.parse(JSON.stringify(nextOverviewPayload));
    },
    async printerCapabilities() { return { forms: [{ name: "A4", size: { width_mm: 210, height_mm: 297 }, roll: false }], duplex: true, color: false, bins: ["Tray 1"] }; },
    async getPrinterSettings() { return {}; },
    async printerForms() { return [{ name: "A4", size: { width_mm: 210, height_mm: 297 } }, { name: "NoSizeForm", size: null }]; },
    async runTestPrint(printer, kind, paper, opts) { testPrints.push({ printer, kind, paper, opts }); return "job-99"; },
    async saveOrigin(origin, autoReprint) { return origin; },
    async exportDiagnostics() { return JSON.stringify({ ok: true, jobs: 2 }); },
  };
  // Patch toastContainer prepend child to have querySelector(".toast-close")
  const origCreate = document.createElement.bind(document);
  document.createElement = (tag) => {
    const el = origCreate(tag);
    el.querySelector = () => new MiniEl("BUTTON");
    return el;
  };

  const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, setTimeout: windowObj.setTimeout, clearTimeout: windowObj.clearTimeout, clearInterval: windowObj.clearInterval, localStorage: windowObj.localStorage, navigator: {} });
  new vm.Script(script).runInContext(ctx);
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));

  // Overview KPIs & XSS escaping
  assert.equal(document.getElementById("kpiPrintersCount").textContent, "2");
  assert.equal(document.getElementById("kpiActiveCount").textContent, "2");
  assert.equal(document.getElementById("kpiDoneCount").textContent, "5");
  assert.equal(document.getElementById("kpiFailedCount").textContent, "1");
  const jobsHtml = document.getElementById("jobsTableBody").innerHTML;
  assert.ok(jobsHtml.includes("&lt;b&gt;Invoice&lt;/b&gt;"), "job report must be HTML-escaped");
  assert.ok(!jobsHtml.includes("<b>Invoice</b>"), "raw HTML must not appear in jobs table");
  const gridHtml = document.getElementById("printersGrid").innerHTML;
  assert.ok(gridHtml.includes("HP &lt;img src=x onerror=alert(1)&gt;"), "printer name must be HTML-escaped");
  assert.ok(!gridHtml.includes("<img src=x"), "raw HTML must not appear in printers grid");
  assert.ok(document.getElementById("envSummary").innerHTML.includes("&lt;script&gt;xss&lt;/script&gt;"));

  // Filtering
  ctx.setJobFilter("done");
  assert.ok(document.getElementById("jobsTableBody").innerHTML.includes("Receipt"));
  assert.ok(!document.getElementById("jobsTableBody").innerHTML.includes("Invoice"));
  ctx.setJobFilter("all");

  // Race-free refreshAll: slow background poll + concurrent manual refresh must NOT drop the manual refresh
  let releasePoll = null;
  resolveSlowOverview = (release) => { releasePoll = release; };
  const callsBefore = overviewCalls;
  const pollP = ctx.refreshAll({ silent: true });
  const manualP = ctx.refreshAll(); // non-silent must coalesce and run after poll
  const dupSilentP = ctx.refreshAll({ silent: true }); // redundant silent poll skips
  assert.equal(await dupSilentP, false);
  releasePoll();
  assert.equal(await pollP, true);
  assert.equal(await manualP, true);
  assert.equal(overviewCalls, callsBefore + 2, "manual refresh must execute a fresh fetch after in-flight poll");

  // Rejected IPC promise marks main-container stale and returns false cleanly
  const savedOverview = windowObj.__NATIVE_PRINT__.appOverview;
  windowObj.__NATIVE_PRINT__.appOverview = async () => { throw new Error("ipc_down"); };
  assert.equal(await ctx.refreshAll({ silent: true }), false);
  assert.equal(document.querySelector(".main-container").classList.contains("is-stale"), true);
  windowObj.__NATIVE_PRINT__.appOverview = savedOverview;

  // Test print: blocks raw print to virtual printer, allows PDF
  assert.equal(await ctx.runPrinterTest("Microsoft Print to PDF", "zpl-label"), false);
  assert.equal(testPrints.length, 0);
  await ctx.runPrinterTest("Microsoft Print to PDF", "pdf-a4");
  assert.equal(testPrints.length, 1);

  // inspectForms: handles null f.size without throwing
  const formsBox = new MiniEl("DIV", "forms-box-0");
  formsBox.classList.add("hidden");
  document.body.appendChild(formsBox);
  await ctx.inspectForms("HP", 0);
  assert.ok(formsBox.innerHTML.includes("210×297mm"));
  assert.ok(formsBox.innerHTML.includes("NoSizeForm"));

  // Diagnostics bundle load
  assert.equal(await ctx.loadDiagnosticsBundle(), true);
  assert.ok(document.getElementById("diagnosticsOutput").textContent.includes('"jobs": 2'));

  // Theme & Language toggle (RTL <-> LTR, aria-label sync, aria-invalid)
  ctx.toggleTheme();
  assert.equal(document.documentElement.getAttribute("data-theme"), "dark");
  assert.equal(storage.get("np_cc_theme"), "dark");
  ctx.toggleLang();
  assert.equal(document.documentElement.lang, "en");
  assert.equal(document.documentElement.dir, "ltr");
  assert.equal(document.getElementById("btnTheme").getAttribute("aria-label"), "Theme");
  assert.equal(document.getElementById("queueSearchInput").getAttribute("aria-label"), "Search jobs…");

  const originInput = document.getElementById("inputOdooOrigin");
  originInput.value = "http://evil.example.com";
  assert.equal(ctx.validateOriginField(), false);
  assert.equal(originInput.getAttribute("aria-invalid"), "true");
  originInput.value = "https://odoo.example.com";
  assert.equal(ctx.validateOriginField(), true);
  assert.equal(originInput.getAttribute("aria-invalid"), "false");
});

test("tabbar.html flows: safe titles, RTL/LTR drag reorder, mid-drag guard, keyboard nav, and visibilitychange polling", async () => {
  const tabHtml = readFileSync(new URL("../dist/tabbar.html", import.meta.url), "utf8");
  const tabScript = tabHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];
  const { windowObj, document, intervals } = createMiniDom(tabHtml);
  const invoked = [];
  const tabsFixture = [
    { label: "t1", title: "Odoo", url: "https://odoo.example.com", active: true, is_pos: false },
    { label: "t2", title: "", url: "https://odoo.example.com/pos", active: false, is_pos: true },
    { label: "t3", title: null, url: "", active: false, is_pos: false },
  ];
  windowObj.__TAURI_INTERNALS__ = {
    async invoke(cmd, args) {
      invoked.push([cmd, args]);
      if (cmd === "tab_list") return tabsFixture;
      return {};
    },
  };
  const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, clearInterval: windowObj.clearInterval, localStorage: windowObj.localStorage });
  new vm.Script(tabScript).runInContext(ctx);
  await new Promise(r => setImmediate(r));

  const strip = document.getElementById("tabs");
  const tabEls = strip.children.filter(c => c.classList.contains("tab"));
  assert.equal(tabEls.length, 3);
  assert.equal(tabEls[1].querySelector(".t-title")?.textContent || tabEls[1].children[1].textContent, "https://odoo.example.com/pos");
  assert.equal(tabEls[2].children[1].textContent, "تبويب جديد");

  // Click tab switch & close button
  tabEls[1].dispatch("click");
  tabEls[1].children[3].dispatch("click");
  await new Promise(r => setImmediate(r));
  assert.ok(invoked.some(([c, a]) => c === "tab_switch" && a.label === "t2"));
  assert.ok(invoked.some(([c, a]) => c === "tab_close" && a.label === "t2"));

  // Keyboard navigation in RTL: ArrowLeft moves to next tab (0 -> 1)
  tabEls[0].focus();
  tabEls[0].dispatch("keydown", { key: "ArrowLeft" });
  assert.equal(document.activeElement, tabEls[1], "RTL ArrowLeft must focus next tab");

  // Drag-and-drop: mid-drag render guard + RTL dropIndexFor
  tabEls[0].dispatch("dragstart", { dataTransfer: { setData() {} } });
  const beforeDragChildren = strip.children.length;
  ctx.render([{ label: "t99", title: "Mutated", url: "", active: true }]);
  assert.equal(strip.children.length, beforeDragChildren, "render() must not wipe tabs mid-drag");
  // Drop t1 on right half of t3 in RTL (frac=0.8 >= 0.5 -> before t3 -> index 1 after removing t1)
  assert.equal(ctx.dropIndexFor(2, 80, { left: 0, width: 100 }), 1);
  // Drop t1 on left half of t3 in RTL (frac=0.2 < 0.5 -> after t3 -> index 2 after removing t1)
  assert.equal(ctx.dropIndexFor(2, 20, { left: 0, width: 100 }), 2);
  tabEls[0].dispatch("dragend");

  // Visibilitychange polling pause/resume
  const callsBeforeHidden = invoked.filter(([c]) => c === "tab_list").length;
  document.hidden = true;
  intervals[0].fn();
  await new Promise(r => setImmediate(r));
  assert.equal(invoked.filter(([c]) => c === "tab_list").length, callsBeforeHidden, "tabbar must pause polling while hidden");
  document.hidden = false;
  document.dispatch("visibilitychange");
  await new Promise(r => setImmediate(r));
  assert.equal(invoked.filter(([c]) => c === "tab_list").length, callsBeforeHidden + 1, "tabbar must refresh on becoming visible");
});

test("boot.html flows: strict origin check, probe success redirect, probe failure down/retry, and initFail", async () => {
  const bootHtml = readFileSync(new URL("../dist/boot.html", import.meta.url), "utf8");
  const bootScript = bootHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];

  // Case 1: valid origin + probe_origin true -> redirects to origin
  {
    const { windowObj, replacedUrls } = createMiniDom(bootHtml);
    windowObj.__TAURI_INTERNALS__ = {
      async invoke(cmd) {
        if (cmd === "app_identity") return { odooOrigin: "https://odoo.example.com" };
        if (cmd === "probe_origin") return true;
      },
    };
    const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, clearInterval: windowObj.clearInterval, setTimeout: windowObj.setTimeout, clearTimeout: windowObj.clearTimeout, localStorage: windowObj.localStorage });
    new vm.Script(bootScript).runInContext(ctx);
    await new Promise(r => setImmediate(r));
    await new Promise(r => setImmediate(r));
    assert.deepEqual(replacedUrls, ["https://odoo.example.com"]);
    assert.equal(ctx.isHttpOrigin("http://"), false);
    assert.equal(ctx.isHttpOrigin("javascript:alert(1)"), false);
    assert.equal(ctx.isHttpOrigin("http://192.168.1.20:8069"), true);
  }

  // Case 2: probe fails -> enters down state, retry button re-probes
  {
    const { windowObj, document, intervals } = createMiniDom(bootHtml);
    let probes = 0;
    windowObj.__TAURI_INTERNALS__ = {
      async invoke(cmd) {
        if (cmd === "app_identity") return { odooOrigin: "http://192.168.1.20:8069" };
        if (cmd === "probe_origin") { probes++; return false; }
      },
    };
    const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, clearInterval: windowObj.clearInterval, setTimeout: windowObj.setTimeout, clearTimeout: windowObj.clearTimeout, localStorage: windowObj.localStorage });
    new vm.Script(bootScript).runInContext(ctx);
    await new Promise(r => setImmediate(r));
    await new Promise(r => setImmediate(r));
    assert.equal(document.querySelector(".card").getAttribute("data-state"), "down");
    assert.equal(intervals.length, 1);
    document.getElementById("retry").dispatch("click");
    await new Promise(r => setImmediate(r));
    assert.equal(probes, 2);
  }

  // Case 3: app_identity rejects -> initFail state
  {
    const { windowObj, document } = createMiniDom(bootHtml);
    windowObj.__TAURI_INTERNALS__ = {
      async invoke() { throw new Error("ipc_unavailable"); },
    };
    const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, clearInterval: windowObj.clearInterval, setTimeout: windowObj.setTimeout, clearTimeout: windowObj.clearTimeout, localStorage: windowObj.localStorage });
    new vm.Script(bootScript).runInContext(ctx);
    await new Promise(r => setImmediate(r));
    assert.equal(document.querySelector(".card").getAttribute("data-state"), "initFail");
  }
});

test("settings.html flows: tab switching + keyboard nav, theme/lang toggle, and app_identity error handling", async () => {
  const settingsHtml = readFileSync(new URL("../dist/settings.html", import.meta.url), "utf8");
  const settingsScript = settingsHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];

  // Case 1: app_identity resolves with origin + theme/lang/tab switching
  {
    const { windowObj, document, storage } = createMiniDom(settingsHtml);
    const invoked = [];
    windowObj.__TAURI_INTERNALS__ = {
      async invoke(cmd) {
        invoked.push(cmd);
        if (cmd === "app_identity") return { appVersion: "1.5.7", odooOrigin: "https://odoo.example.com" };
        return {};
      },
    };
    const ctx = vm.createContext({ ...windowObj, localStorage: windowObj.localStorage });
    new vm.Script(settingsScript).runInContext(ctx);
    await new Promise(r => setImmediate(r));
    assert.equal(document.getElementById("appVersion").textContent, "v1.5.7");
    assert.equal(document.getElementById("btnOpenOdoo").disabled, false);

    // Keyboard ArrowDown moves from About -> Prefs
    document.getElementById("tab-about").dispatch("keydown", { key: "ArrowDown" });
    assert.equal(document.getElementById("tab-prefs").getAttribute("aria-selected"), "true");
    assert.equal(document.getElementById("page-prefs").classList.contains("active"), true);

    // Theme & language toggle
    document.getElementById("btnThemeDark").dispatch("click");
    assert.equal(document.documentElement.getAttribute("data-theme"), "dark");
    assert.equal(document.getElementById("btnThemeDark").getAttribute("aria-pressed"), "true");
    document.getElementById("btnLangEn").dispatch("click");
    assert.equal(document.documentElement.lang, "en");
    assert.equal(document.documentElement.dir, "ltr");
    assert.equal(document.title, "Yaseir Print — Settings");
    assert.equal(storage.get("np_cc_lang"), "en");

    // Support action button opens Control Center / Diagnostics
    document.getElementById("btnOpenDiagnostics").dispatch("click");
    assert.ok(invoked.includes("open_control_center"), "btnOpenDiagnostics must invoke open_control_center");
  }

  // Case 2: app_identity rejects -> btnOpenOdoo gracefully disabled
  {
    const { windowObj, document } = createMiniDom(settingsHtml);
    windowObj.__TAURI_INTERNALS__ = {
      async invoke() { throw new Error("bridge_down"); },
    };
    const ctx = vm.createContext({ ...windowObj, localStorage: windowObj.localStorage });
    new vm.Script(settingsScript).runInContext(ctx);
    await new Promise(r => setImmediate(r));
    assert.equal(document.getElementById("btnOpenOdoo").disabled, true);
  }
});

test("queue search input is debounced (~120ms) while filter clicks render immediately", async () => {
  const { windowObj, document, timers, MiniEl } = createMiniDom(html);
  windowObj.__NATIVE_PRINT__ = {
    async appOverview() {
      return {
        appVersion: "1.5.7",
        printers: [{ name: "HP", kind: "local", is_default: true, status: "ready", port: "USB001", driver: "HP" }],
        defaultPrinter: "HP",
        stateCounts: [["done", 2], ["failed", 1]],
        pdfiumReady: true,
        webviewVersion: "120.0",
        environment: { es2022: true, localStorage: true, cookies: true, webSocket: true, secureContext: true, errors: [] },
        odooOrigin: "https://odoo.example.com",
        lockedByPolicy: false,
        autoReprint: false,
        recentJobs: [
          { id: "j-1", report: "Invoice Alpha", printer: "HP", pages: 2, bytes: 2048, state: "failed", attempts: 2, error_code: "ERR" },
          { id: "j-2", report: "Receipt Beta", printer: "HP", pages: 1, bytes: 512, state: "done", attempts: 1, error_code: "" },
          { id: "j-3", report: "Shipping Gamma", printer: "HP", pages: 1, bytes: 1024, state: "done", attempts: 1, error_code: "" },
        ],
      };
    },
  };
  const origCreate = document.createElement.bind(document);
  document.createElement = (tag) => {
    const el = origCreate(tag);
    el.querySelector = () => new MiniEl("BUTTON");
    return el;
  };
  const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, setTimeout: windowObj.setTimeout, clearTimeout: windowObj.clearTimeout, clearInterval: windowObj.clearInterval, localStorage: windowObj.localStorage, navigator: {} });
  new vm.Script(script).runInContext(ctx);
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));

  const tbody = document.getElementById("jobsTableBody");
  const searchInput = document.getElementById("queueSearchInput");
  let renderWrites = 0;
  const desc = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(tbody), "innerHTML");
  Object.defineProperty(tbody, "innerHTML", {
    get() { return desc.get.call(this); },
    set(v) { renderWrites++; desc.set.call(this, v); },
  });

  // Rapid typing should coalesce into a single 120ms debounced render
  const timersBefore = timers.length;
  for (const val of ["R", "Re", "Rec"]) {
    searchInput.value = val;
    searchInput.dispatch("input", { target: searchInput });
  }
  assert.equal(renderWrites, 0, "rapid typing must not synchronously re-render jobs table");
  const searchTimers = timers.slice(timersBefore);
  assert.equal(searchTimers.length, 3, "expected one timer scheduled per keystroke");
  assert.equal(searchTimers[0].cleared, true, "first keystroke timer must be cleared");
  assert.equal(searchTimers[1].cleared, true, "second keystroke timer must be cleared");
  assert.equal(searchTimers[2].cleared, false, "latest keystroke timer must remain active");
  assert.equal(searchTimers[2].ms, 120, "debounce delay should be 120ms");

  // Firing the debounced timer renders once with the latest query
  searchTimers[2].fn();
  assert.equal(renderWrites, 1, "firing debounced timer must render jobs table once");
  assert.ok(tbody.innerHTML.includes("Receipt Beta"));
  assert.ok(!tbody.innerHTML.includes("Invoice Alpha"));

  // Immediate filter pill click must render synchronously and cancel any pending search timer
  searchInput.value = "";
  searchInput.dispatch("input", { target: searchInput });
  const pendingTimer = timers[timers.length - 1];
  assert.equal(pendingTimer.cleared, false);
  const donePill = document.querySelectorAll(".filter-pill").find(p => p.getAttribute("data-filter") === "done");
  assert.ok(donePill, "done filter pill must exist");
  donePill.closest = () => donePill;
  document.dispatch("click", { target: donePill });
  assert.equal(renderWrites, 2, "filter pill click must render immediately without waiting for debounce");
  assert.equal(pendingTimer.cleared, true, "filter click must cancel pending search debounce timer");
  assert.ok(tbody.innerHTML.includes("Receipt Beta"));
  assert.ok(tbody.innerHTML.includes("Shipping Gamma"));
  assert.ok(!tbody.innerHTML.includes("Invoice Alpha"));
});

test("copy-diagnostics uses navigator.clipboard.writeText, falls back to execCommand, and shows localized toasts", async () => {
  const { windowObj, document, MiniEl } = createMiniDom(html);
  let exportCalls = 0;
  windowObj.__NATIVE_PRINT__ = {
    async appOverview() {
      return {
        appVersion: "1.5.7", printers: [], defaultPrinter: null, stateCounts: [],
        pdfiumReady: true, webviewVersion: "120.0", environment: { errors: [] },
        odooOrigin: "https://odoo.example.com", lockedByPolicy: false, autoReprint: false, recentJobs: [],
      };
    },
    async exportDiagnostics() { exportCalls++; return JSON.stringify({ ok: true, jobs: 2 }); },
  };
  const origCreate = document.createElement.bind(document);
  document.createElement = (tag) => {
    const el = origCreate(tag);
    el.querySelector = () => new MiniEl("BUTTON");
    return el;
  };
  const copiedTexts = [];
  let clipboardImpl = async (t) => { copiedTexts.push(t); };
  let execCommandResult = true;
  const execCommands = [];
  document.execCommand = (cmd) => { execCommands.push(cmd); return execCommandResult; };

  const ctx = vm.createContext({
    ...windowObj,
    setInterval: windowObj.setInterval,
    setTimeout: windowObj.setTimeout,
    clearTimeout: windowObj.clearTimeout,
    clearInterval: windowObj.clearInterval,
    localStorage: windowObj.localStorage,
    navigator: { clipboard: { writeText: (t) => clipboardImpl(t) } },
  });
  new vm.Script(script).runInContext(ctx);
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));

  const btnCopy = document.getElementById("btnCopyDiagnostics");
  assert.ok(btnCopy, "btnCopyDiagnostics must exist");
  assert.equal(btnCopy.getAttribute("data-action"), "copy-diagnostics");
  assert.equal(btnCopy.getAttribute("type"), "button");

  // 1. One-click copy via delegated click auto-loads diagnostics and writes JSON to clipboard
  btnCopy.closest = () => btnCopy;
  document.dispatch("click", { target: btnCopy });
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));
  assert.equal(exportCalls, 1, "one-click copy should auto-load diagnostics bundle");
  assert.equal(copiedTexts.length, 1);
  assert.ok(copiedTexts[0].includes('"jobs": 2'));
  const toasts = document.getElementById("toastContainer");
  assert.ok(toasts.children[0].className.includes("toast-success"));
  assert.ok(toasts.children[0].innerHTML.includes("تم نسخ التقرير"));

  // 2. Fallback to document.execCommand("copy") in English when clipboard API rejects
  ctx.toggleLang();
  clipboardImpl = async () => { throw new Error("denied"); };
  await ctx.copyDiagnosticsBundle();
  assert.deepEqual(execCommands, ["copy"]);
  assert.ok(toasts.children[0].className.includes("toast-success"));
  assert.ok(toasts.children[0].innerHTML.includes("Report copied"));

  // 3. Graceful error toast when both clipboard API and execCommand fail
  execCommandResult = false;
  await ctx.copyDiagnosticsBundle();
  assert.ok(toasts.children[0].className.includes("toast-error"));
  assert.ok(toasts.children[0].innerHTML.includes("Automatic copy failed"));
});

test("Yaseir Precision v2: token-only CSS, RTL Arabic typography, semantic KPI/status pills, and actionable empty states", async () => {
  const styleMatch = html.match(/<style\b[^>]*>([\s\S]*?)<\/style>/i);
  assert.ok(styleMatch, "index.html must contain a <style> block");
  const styleCss = styleMatch[1];

  // 1. Strict theme.css token discipline: zero hardcoded hex colors in index.html CSS
  const hexColors = [...styleCss.matchAll(/#[0-9a-f]{3,8}\b/gi)].map(m => m[0]);
  assert.deepEqual(hexColors, [], "index.html CSS must use theme.css tokens, not hardcoded hex colors");

  // 2. RTL Arabic typography: zero letter-spacing on Arabic headings/buttons/tabs & comfortable line-height
  assert.match(styleCss, /\[dir="rtl"\]\s+body\s*\{[^}]*line-height:\s*1\.65/);
  assert.match(styleCss, /\[dir="rtl"\]\s+\.brand-title[\s\S]*?letter-spacing:\s*0;/);
  assert.match(styleCss, /\.printer-card:focus-within/);
  assert.match(styleCss, /tbody tr\s*\{\s*transition:\s*background-color var\(--dur-fast\) var\(--ease-out\)/);

  // 3. KPI semantic accent classes + tabular-nums
  for (const cls of ["kpi-info", "kpi-warn", "kpi-ok", "kpi-err"]) {
    assert.ok(html.includes(`kpi-card ${cls}`), "missing KPI semantic class: " + cls);
    assert.ok(styleCss.includes(`.kpi-card.${cls}::before`), "missing KPI accent hairline CSS for: " + cls);
  }
  assert.equal([...html.matchAll(/class="kpi-value tabular-nums"/g)].length, 4, "all 4 KPI values must use tabular-nums");

  // 4. Status pill differentiation (ready vs printing vs offline/error) and actionable empty states
  const { windowObj, document, MiniEl } = createMiniDom(html);
  windowObj.__NATIVE_PRINT__ = {
    async appOverview() {
      return {
        appVersion: "1.5.7", printers: [], defaultPrinter: null, stateCounts: [],
        pdfiumReady: true, webviewVersion: "120.0", environment: { errors: [] },
        odooOrigin: "https://odoo.example.com", lockedByPolicy: false, autoReprint: false, recentJobs: [],
      };
    },
  };
  const origCreate = document.createElement.bind(document);
  document.createElement = (tag) => {
    const el = origCreate(tag);
    el.querySelector = () => new MiniEl("BUTTON");
    return el;
  };
  const ctx = vm.createContext({
    ...windowObj,
    setInterval: windowObj.setInterval,
    setTimeout: windowObj.setTimeout,
    clearTimeout: windowObj.clearTimeout,
    clearInterval: windowObj.clearInterval,
    localStorage: windowObj.localStorage,
    navigator: {},
  });
  new vm.Script(script).runInContext(ctx);
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));

  assert.match(ctx.printerStatusBadge("ready"), /badge-ok/);
  assert.match(ctx.printerStatusBadge("printing"), /badge-info/);
  assert.match(ctx.printerStatusBadge("offline"), /badge-err/);
  assert.match(ctx.printerStatusBadge("error"), /badge-err/);
  assert.match(ctx.printerStatusBadge("paused"), /badge-warn/);

  const emptyJobs = document.getElementById("jobsTableBody").innerHTML;
  assert.ok(emptyJobs.includes("empty-state-inner"), "queue empty state must use calm framed wrapper");
  assert.ok(emptyJobs.includes('data-action="switch-tab"'), "queue empty state must provide a clear next action");

  const emptyPrinters = document.getElementById("printersGrid").innerHTML;
  assert.ok(emptyPrinters.includes("empty-state-inner"), "printers empty state must use calm framed wrapper");
  assert.ok(emptyPrinters.includes('data-action="manual-refresh"'), "printers empty state must provide a rescan action");
});

test("tabbar.html pointer-event drag-and-drop reorders tabs and refresh() drains pending np_tab_cmd", async () => {
  const tabHtml = readFileSync(new URL("../dist/tabbar.html", import.meta.url), "utf8");
  const tabScript = tabHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];
  const { windowObj, document } = createMiniDom(tabHtml);
  const invoked = [];
  const tabsFixture = [
    { label: "t1", title: "Odoo", url: "https://odoo.example.com", active: true, is_pos: false },
    { label: "t2", title: "POS", url: "https://odoo.example.com/pos", active: false, is_pos: true },
  ];
  windowObj.__TAURI_INTERNALS__ = {
    async invoke(cmd, args) {
      invoked.push([cmd, args]);
      if (cmd === "tab_list") return tabsFixture;
      return {};
    },
  };
  const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, clearInterval: windowObj.clearInterval, localStorage: windowObj.localStorage });
  new vm.Script(tabScript).runInContext(ctx);
  await new Promise(r => setImmediate(r));

  const strip = document.getElementById("tabs");
  const tabEls = strip.children.filter(c => c.classList.contains("tab"));
  tabEls[0].getBoundingClientRect = () => ({ left: 100, right: 200, width: 100 });
  tabEls[1].getBoundingClientRect = () => ({ left: 0, right: 100, width: 100 });

  // Pointer drag t1 over left half of t2 in RTL -> reorders to index 1
  tabEls[0].dispatch("pointerdown", { button: 0, clientX: 150, pointerId: 1 });
  windowObj.dispatch("pointermove", { clientX: 20 });
  windowObj.dispatch("pointerup", {});
  await new Promise(r => setImmediate(r));
  assert.ok(invoked.some(([c, a]) => c === "tab_move" && a.label === "t1" && a.toIndex === 1), "pointer drag must invoke tab_move");

  // Pending cross-tab command in localStorage executes on refresh() even without a storage event
  windowObj.localStorage.setItem("np_tab_cmd", JSON.stringify({ action: "switch", label: "t2", ts: Date.now() }));
  ctx.refresh();
  await new Promise(r => setImmediate(r));
  assert.ok(invoked.some(([c, a]) => c === "tab_switch" && a.label === "t2"), "refresh() must drain np_tab_cmd");
  assert.equal(windowObj.localStorage.getItem("np_tab_cmd"), null, "np_tab_cmd must be cleared after execution");

  // Closing a tab prunes stale label from np_pinned_tabs and np_tab_groups
  windowObj.localStorage.setItem("np_pinned_tabs", JSON.stringify(["t1", "t2"]));
  windowObj.localStorage.setItem("np_tab_groups", JSON.stringify({ t1: "blue", t2: "green" }));
  windowObj.localStorage.setItem("np_tab_cmd", JSON.stringify({ action: "close", label: "t2", ts: Date.now() + 1 }));
  ctx.refresh();
  await new Promise(r => setImmediate(r));
  assert.deepEqual(JSON.parse(windowObj.localStorage.getItem("np_pinned_tabs")), ["t1"]);
  assert.deepEqual(JSON.parse(windowObj.localStorage.getItem("np_tab_groups")), { t1: "blue" });
});

test("tabbar.html pointercancel rolls back drag without invoking tab_move and isSafeTabUrl rejects unsafe URLs", async () => {
  const tabHtml = readFileSync(new URL("../dist/tabbar.html", import.meta.url), "utf8");
  const tabScript = tabHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];
  const { windowObj, document } = createMiniDom(tabHtml);
  const invoked = [];
  const tabsFixture = [
    { label: "t1", title: "Odoo", url: "https://odoo.example.com", active: true, is_pos: false },
    { label: "t2", title: "POS", url: "https://odoo.example.com/pos", active: false, is_pos: true },
  ];
  windowObj.__TAURI_INTERNALS__ = {
    async invoke(cmd, args) {
      invoked.push([cmd, args]);
      if (cmd === "tab_list") return tabsFixture;
      return {};
    },
  };
  const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, clearInterval: windowObj.clearInterval, setTimeout: windowObj.setTimeout, clearTimeout: windowObj.clearTimeout, localStorage: windowObj.localStorage });
  new vm.Script(tabScript).runInContext(ctx);
  await new Promise(r => setImmediate(r));

  const strip = document.getElementById("tabs");
  const tabEls = strip.children.filter(c => c.classList.contains("tab"));
  tabEls[0].getBoundingClientRect = () => ({ left: 100, right: 200, width: 100 });
  tabEls[1].getBoundingClientRect = () => ({ left: 0, right: 100, width: 100 });

  // Pointer drag started and moved, then canceled via pointercancel -> must NOT invoke tab_move
  tabEls[0].dispatch("pointerdown", { button: 0, clientX: 150, pointerId: 1 });
  windowObj.dispatch("pointermove", { clientX: 20 });
  assert.equal(tabEls[0].classList.contains("dragging"), true, "drag state active before pointercancel");
  windowObj.dispatch("pointercancel", {});
  await new Promise(r => setImmediate(r));
  assert.equal(tabEls[0].classList.contains("dragging"), false, "pointercancel clears dragging class");
  assert.equal(invoked.some(([c]) => c === "tab_move"), false, "pointercancel must not invoke tab_move");

  // isSafeTabUrl allows empty / http(s) and rejects javascript:, credentialed, and non-http(s) URLs
  assert.equal(ctx.isSafeTabUrl(""), true);
  assert.equal(ctx.isSafeTabUrl("https://odoo.example.com/odoo"), true);
  assert.equal(ctx.isSafeTabUrl("http://192.168.1.20:8069/pos"), true);
  assert.equal(ctx.isSafeTabUrl("javascript:alert(1)"), false);
  assert.equal(ctx.isSafeTabUrl("JAVASCRIPT:void(0)"), false);
  assert.equal(ctx.isSafeTabUrl("https://user:pass@odoo.example.com"), false);
  assert.equal(ctx.isSafeTabUrl("https://user@odoo.example.com"), false);
  assert.equal(ctx.isSafeTabUrl("file:///etc/passwd"), false);
  assert.equal(ctx.isSafeTabUrl("data:text/html,<script>alert(1)</script>"), false);

  // Command path enforcement: open action only opens safe URLs
  let ts = Date.now();
  for (const url of ["javascript:alert(1)", "https://user:pass@odoo.example.com/x", "https://odoo.example.com/safe"]) {
    windowObj.localStorage.setItem("np_tab_cmd", JSON.stringify({ action: "open", url, ts: ++ts }));
    ctx.refresh();
    await new Promise(r => setImmediate(r));
  }
  const openedUrls = invoked.filter(([c]) => c === "tab_open").map(([, a]) => a.url);
  assert.deepEqual(openedUrls, ["https://odoo.example.com/safe"], "unsafe URLs must never reach tab_open");
});

test("boot.html retry resets down message back to connecting state while re-probing", async () => {
  const bootHtml = readFileSync(new URL("../dist/boot.html", import.meta.url), "utf8");
  const bootScript = bootHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];
  const { windowObj, document, replacedUrls } = createMiniDom(bootHtml);

  let probes = 0;
  let resolveSecondProbe = null;
  windowObj.__TAURI_INTERNALS__ = {
    async invoke(cmd) {
      if (cmd === "app_identity") return { odooOrigin: "http://192.168.1.20:8069" };
      if (cmd === "probe_origin") {
        probes++;
        if (probes === 1) return false;
        return new Promise((res) => { resolveSecondProbe = res; });
      }
    },
  };
  const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, clearInterval: windowObj.clearInterval, setTimeout: windowObj.setTimeout, clearTimeout: windowObj.clearTimeout, localStorage: windowObj.localStorage });
  new vm.Script(bootScript).runInContext(ctx);
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));

  const card = document.querySelector(".card");
  const title = document.getElementById("title");
  const msg = document.getElementById("msg");
  const retryBtn = document.getElementById("retry");
  assert.equal(card.getAttribute("data-state"), "down");
  assert.ok(msg.textContent.includes("لا استجابة من "), "failed probe shows down message");

  // Trigger retry and inspect state while second probe is still pending
  retryBtn.dispatch("click");
  assert.equal(probes, 2);
  assert.equal(card.getAttribute("data-state"), "connecting");
  assert.equal(title.textContent, "جارٍ الاتصال بـ Odoo");
  assert.equal(msg.textContent, "تجهيز الجلسة… ", "retry must reset stale down message to preparing text");
  assert.equal(msg.children[0]?.textContent, "http://192.168.1.20:8069", "retry keeps origin span in message");
  assert.equal(retryBtn.disabled, true, "retry button disabled while probing");

  resolveSecondProbe(true);
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));
  assert.equal(retryBtn.disabled, false);
  assert.deepEqual(replacedUrls, ["http://192.168.1.20:8069"]);
});

test("index.html Print Preview modal opens per printer profile with dimensions, runs test print, and closes cleanly", async () => {
  const { windowObj, document, MiniEl } = createMiniDom(html);
  const testPrints = [];
  windowObj.__NATIVE_PRINT__ = {
    async appOverview() {
      return {
        appVersion: "1.5.7",
        printers: [
          { name: "Epson TM-T88V 80mm", kind: "local", is_default: true, status: "ready", port: "USB001", driver: "EPSON TM-T88V", printer_class: "thermal", protocol: "escpos" },
          { name: "HP LaserJet Pro", kind: "local", is_default: false, status: "ready", port: "USB002", driver: "HP Universal", printer_class: "office", protocol: "gdi" },
        ],
        defaultPrinter: "Epson TM-T88V 80mm",
        stateCounts: [],
        pdfiumReady: true,
        webviewVersion: "120.0",
        environment: { errors: [] },
        odooOrigin: "https://odoo.example.com",
        lockedByPolicy: false,
        autoReprint: false,
        recentJobs: [],
      };
    },
    async printerCapabilities() { return { forms: [{ name: "A4", size: { width_mm: 210, height_mm: 297 }, roll: false }], duplex: false, color: false, bins: [] }; },
    async getPrinterSettings() { return {}; },
    async runTestPrint(printer, kind, paper, opts) { testPrints.push({ printer, kind, paper, opts }); return "job-pv-1"; },
  };
  const origCreate = document.createElement.bind(document);
  document.createElement = (tag) => {
    const el = origCreate(tag);
    el.querySelector = () => new MiniEl("BUTTON");
    return el;
  };
  const ctx = vm.createContext({
    ...windowObj,
    setInterval: windowObj.setInterval,
    setTimeout: windowObj.setTimeout,
    clearTimeout: windowObj.clearTimeout,
    clearInterval: windowObj.clearInterval,
    localStorage: windowObj.localStorage,
    navigator: {},
  });
  new vm.Script(script).runInContext(ctx);
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));

  const gridHtml = document.getElementById("printersGrid").innerHTML;
  assert.ok(gridHtml.includes('data-action="preview-printer"'), "printer cards must include preview-printer button");

  const clickAction = (action, attrs = {}) => {
    const btn = new MiniEl("BUTTON");
    btn.setAttribute("data-action", action);
    for (const [k, v] of Object.entries(attrs)) btn.setAttribute(k, String(v));
    btn.closest = () => btn;
    document.dispatch("click", { target: btn });
  };

  // Open thermal printer preview
  const overlay = document.getElementById("previewOverlay");
  assert.equal(overlay.classList.contains("hidden"), true, "previewOverlay starts hidden");
  clickAction("preview-printer", { "data-printer": "Epson TM-T88V 80mm" });
  assert.equal(overlay.classList.contains("hidden"), false, "previewOverlay becomes visible");
  assert.equal(document.getElementById("previewPrinterName").textContent, "Epson TM-T88V 80mm");
  assert.equal(document.getElementById("previewPaperMeta").textContent, "80×200mm");
  assert.ok(document.getElementById("previewModeMeta").textContent.includes("203 DPI"));
  assert.equal(document.getElementById("previewSheet").classList.contains("sheet-80"), true);

  // Trigger Test Print from preview modal
  clickAction("preview-test-print");
  await new Promise(r => setImmediate(r));
  assert.equal(testPrints.length, 1);
  assert.equal(testPrints[0].printer, "Epson TM-T88V 80mm");
  assert.equal(testPrints[0].kind, "thermal-80mm");

  // Close via preview-close
  clickAction("preview-close");
  assert.equal(overlay.classList.contains("hidden"), true, "preview-close hides previewOverlay");

  // Open office printer preview and close via Escape key
  clickAction("preview-printer", { "data-printer": "HP LaserJet Pro" });
  assert.equal(overlay.classList.contains("hidden"), false);
  assert.equal(document.getElementById("previewPrinterName").textContent, "HP LaserJet Pro");
  assert.equal(document.getElementById("previewPaperMeta").textContent, "210×297mm");
  assert.equal(document.getElementById("previewSheet").classList.contains("sheet-a4"), true);
  document.dispatch("keydown", { key: "Escape" });
  assert.equal(overlay.classList.contains("hidden"), true, "Escape key hides previewOverlay");
});

test("modal keyboard isolation: Escape only closes topmost confirmOverlay, Ctrl+K respects modals, preview Tab traps focus, and previewTestPrintBtn locks in-flight", async () => {
  const { windowObj, document, MiniEl } = createMiniDom(html);
  let resolvePrint = null;
  windowObj.__NATIVE_PRINT__ = {
    async appOverview() {
      return {
        appVersion: "1.5.7",
        printers: [
          { name: "HP LaserJet Pro", kind: "local", is_default: true, status: "ready", port: "USB001", driver: "HP", printer_class: "office", protocol: "gdi" },
        ],
        defaultPrinter: "HP LaserJet Pro",
        stateCounts: [],
        pdfiumReady: true,
        webviewVersion: "120.0",
        environment: { errors: [] },
        odooOrigin: "https://odoo.example.com",
        lockedByPolicy: false,
        autoReprint: false,
        recentJobs: [],
      };
    },
    async printerCapabilities() { return { forms: [], duplex: false, color: false, bins: [] }; },
    async getPrinterSettings() { return {}; },
    async runTestPrint() { return new Promise((res) => { resolvePrint = () => res("job-lock-1"); }); },
  };
  const origCreate = document.createElement.bind(document);
  document.createElement = (tag) => {
    const el = origCreate(tag);
    el.querySelector = () => new MiniEl("BUTTON");
    return el;
  };
  const ctx = vm.createContext({
    ...windowObj,
    setInterval: windowObj.setInterval,
    setTimeout: windowObj.setTimeout,
    clearTimeout: windowObj.clearTimeout,
    clearInterval: windowObj.clearInterval,
    localStorage: windowObj.localStorage,
    navigator: {},
  });
  new vm.Script(script).runInContext(ctx);
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));

  // 1. Initial roving tabindex on navTabs
  assert.equal(document.getElementById("nav-tab-dashboard").getAttribute("tabindex"), "0");
  assert.equal(document.getElementById("nav-tab-printers").getAttribute("tabindex"), "-1");

  // 2. Escape when confirmOverlay is open over previewOverlay must ONLY close confirmOverlay
  const pvOv = document.getElementById("previewOverlay");
  const cfOv = document.getElementById("confirmOverlay");
  ctx.openPrintPreview("HP LaserJet Pro");
  const confirmP = ctx.confirmDialog("Sure?", { danger: true });
  assert.equal(pvOv.classList.contains("hidden"), false);
  assert.equal(cfOv.classList.contains("hidden"), false);

  // Escape closes ONLY confirmOverlay, leaving previewOverlay open
  document.dispatch("keydown", { key: "Escape", preventDefault() {} });
  assert.equal(await confirmP, false);
  assert.equal(cfOv.classList.contains("hidden"), true, "confirmOverlay closed on Escape");
  assert.equal(pvOv.classList.contains("hidden"), false, "previewOverlay must stay open when Escape dismisses confirmOverlay");

  // 3. Tab inside previewOverlay traps focus between previewCloseBtn and previewTestPrintBtn
  const pvClose = document.getElementById("previewCloseBtn");
  const pvPrint = document.getElementById("previewTestPrintBtn");
  pvPrint.focus();
  let tabPrevented = false;
  document.dispatch("keydown", { key: "Tab", shiftKey: false, preventDefault() { tabPrevented = true; } });
  assert.equal(tabPrevented, true, "Tab on last button in previewOverlay must wrap");
  assert.equal(document.activeElement, pvClose);

  // 4. previewTestPrintBtn is disabled while runPrinterTest is in flight
  const printP = ctx.runPrinterTest("HP LaserJet Pro", "pdf-a4");
  assert.equal(pvPrint.disabled, true, "previewTestPrintBtn must be disabled while test print is in flight");
  resolvePrint();
  await printP;
  assert.equal(pvPrint.disabled, false, "previewTestPrintBtn must re-enable after test print finishes");
});

test("steady-state 5s polls perform zero redundant DOM writes while state, filter, search, language, and error recovery re-render immediately", async () => {
  const { windowObj, document, timers, MiniEl } = createMiniDom(html);
  const writes = [];
  const wrapEl = (el) => {
    if (!el || el.__instrumented) return el;
    el.__instrumented = true;
    const tag = () => el.id || el.tagName;
    const origAdd = el.classList.add.bind(el.classList);
    const origRem = el.classList.remove.bind(el.classList);
    const origTog = el.classList.toggle.bind(el.classList);
    el.classList.add = (...c) => { writes.push(`${tag()}.classList.add(${c.join(",")})`); return origAdd(...c); };
    el.classList.remove = (...c) => { writes.push(`${tag()}.classList.remove(${c.join(",")})`); return origRem(...c); };
    el.classList.toggle = (c, f) => { writes.push(`${tag()}.classList.toggle(${c},${f})`); return origTog(c, f); };
    const origSetAttr = el.setAttribute.bind(el);
    el.setAttribute = (k, v) => { writes.push(`${tag()}.setAttribute(${k}=${v})`); return origSetAttr(k, v); };
    const origHtmlDesc = Object.getOwnPropertyDescriptor(MiniEl.prototype, "innerHTML");
    Object.defineProperty(el, "innerHTML", {
      get() { return origHtmlDesc.get.call(this); },
      set(v) { writes.push(`${tag()}.innerHTML`); origHtmlDesc.set.call(this, v); },
    });
    const origTextDesc = Object.getOwnPropertyDescriptor(MiniEl.prototype, "textContent");
    Object.defineProperty(el, "textContent", {
      get() { return origTextDesc.get.call(this); },
      set(v) { writes.push(`${tag()}.textContent`); origTextDesc.set.call(this, v); },
    });
    let disabledVal = el.disabled;
    Object.defineProperty(el, "disabled", {
      get() { return disabledVal; },
      set(v) { writes.push(`${tag()}.disabled=${!!v}`); disabledVal = !!v; },
    });
    return el;
  };
  const walkWrap = (node) => {
    wrapEl(node);
    for (const c of node.children || []) walkWrap(c);
  };
  walkWrap(document.body);

  let payload = {
    appVersion: "1.5.7",
    printers: [
      { name: "HP LaserJet", kind: "local", is_default: true, status: "ready", port: "USB001", driver: "HP" },
      { name: "Epson TM-T88", kind: "local", is_default: false, status: "ready", port: "192.168.1.50", driver: "EPSON" },
    ],
    defaultPrinter: "HP LaserJet",
    stateCounts: [["queued", 1], ["done", 5], ["failed", 0]],
    pdfiumReady: true,
    webviewVersion: "120.0",
    environment: { es2022: true, localStorage: true, cookies: true, webSocket: true, secureContext: true, errors: [] },
    odooOrigin: "https://odoo.example.com",
    lockedByPolicy: false,
    autoReprint: false,
    recentJobs: [
      { id: "j-1", report: "Invoice", printer: "HP LaserJet", pages: 2, bytes: 2048, state: "done", attempts: 1, error_code: "" },
      { id: "j-2", report: "Receipt", printer: "Epson TM-T88", pages: 1, bytes: 512, state: "queued", attempts: 0, error_code: "" },
    ],
  };
  let failOverview = false;
  windowObj.__NATIVE_PRINT__ = {
    async appOverview() {
      if (failOverview) throw new Error("ipc_down");
      return JSON.parse(JSON.stringify(payload));
    },
    async printerCapabilities() { return { forms: [{ name: "A4", size: { width_mm: 210, height_mm: 297 } }], duplex: true, color: false, bins: ["Tray 1"] }; },
    async getPrinterSettings() { return {}; },
  };

  const ctx = vm.createContext({
    ...windowObj,
    setInterval: windowObj.setInterval,
    setTimeout: windowObj.setTimeout,
    clearTimeout: windowObj.clearTimeout,
    clearInterval: windowObj.clearInterval,
    localStorage: windowObj.localStorage,
    navigator: {},
  });
  new vm.Script(script).runInContext(ctx);
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));

  // 1. Five consecutive identical steady-state polls must perform 0 DOM writes
  writes.length = 0;
  for (let i = 0; i < 5; i++) {
    assert.equal(await ctx.refreshAll({ silent: true }), true);
  }
  assert.deepEqual(writes, [], "identical steady-state polls must perform zero redundant DOM writes");

  // 2. Job filter change re-renders jobsTableBody immediately without touching printersGrid
  writes.length = 0;
  ctx.setJobFilter("done");
  assert.ok(writes.includes("jobsTableBody.innerHTML"), "filter change must re-render jobsTableBody");
  assert.ok(!writes.includes("printersGrid.innerHTML"), "filter change must not re-render printersGrid");
  ctx.setJobFilter("all");

  // 3. Queue search change re-renders jobsTableBody after debounce
  writes.length = 0;
  const searchEl = document.getElementById("queueSearchInput");
  searchEl.value = "Receipt";
  searchEl.dispatch("input", { target: searchEl });
  const searchTimer = timers.find(t => !t.cleared && t.ms === 120);
  assert.ok(searchTimer, "expected debounced search timer");
  searchTimer.fn();
  assert.ok(writes.includes("jobsTableBody.innerHTML"), "search change must re-render jobsTableBody");
  searchEl.value = "";
  searchEl.dispatch("input", { target: searchEl });
  timers.filter(t => !t.cleared && t.ms === 120).at(-1).fn();

  // 4. Printer status change re-renders printersGrid without rewriting jobsTableBody
  writes.length = 0;
  payload.printers[0].status = "offline";
  await ctx.refreshAll({ silent: true });
  assert.ok(writes.includes("printersGrid.innerHTML"), "printer status change must re-render printersGrid");
  assert.ok(!writes.includes("jobsTableBody.innerHTML"), "printer status change must not re-render jobsTableBody");

  // 5. KPI / job change updates KPIs and jobsTableBody without rewriting policyStatusBadge
  writes.length = 0;
  payload.stateCounts = [["queued", 0], ["done", 6], ["failed", 0]];
  payload.recentJobs[1].state = "done";
  await ctx.refreshAll({ silent: true });
  assert.equal(document.getElementById("kpiDoneCount").textContent, "6");
  assert.ok(writes.includes("jobsTableBody.innerHTML"), "job state change must re-render jobsTableBody");
  assert.ok(!writes.includes("policyStatusBadge.innerHTML"), "unchanged policy badge must not rewrite innerHTML");

  // 6. Language switch re-renders printersGrid, jobsTableBody, envSummary, and policyStatusBadge immediately
  writes.length = 0;
  ctx.toggleLang();
  assert.ok(writes.includes("printersGrid.innerHTML"), "language switch must re-render printersGrid");
  assert.ok(writes.includes("jobsTableBody.innerHTML"), "language switch must re-render jobsTableBody");
  assert.ok(writes.includes("envSummary.innerHTML"), "language switch must re-render envSummary");
  assert.ok(writes.includes("policyStatusBadge.innerHTML"), "language switch must re-render policyStatusBadge");

  // 7. Recovery after an IPC error clears is-stale and hides originAlertBanner even when payload is unchanged
  failOverview = true;
  assert.equal(await ctx.refreshAll({ silent: true }), false);
  assert.equal(document.querySelector(".main-container").classList.contains("is-stale"), true);
  assert.equal(document.getElementById("originAlertBanner").classList.contains("hidden"), false);
  failOverview = false;
  assert.equal(await ctx.refreshAll({ silent: true }), true);
  assert.equal(document.querySelector(".main-container").classList.contains("is-stale"), false);
  assert.equal(document.getElementById("originAlertBanner").classList.contains("hidden"), true);
});

test("prefers-reduced-motion is enforced in theme.css and respected across all desktop pages", () => {
  const css = readFileSync(new URL("../dist/theme.css", import.meta.url), "utf8");
  const rmBlock = css.match(/@media\s*\(\s*prefers-reduced-motion\s*:\s*reduce\s*\)\s*\{([\s\S]*?)\n\}/)?.[1] || "";
  assert.ok(rmBlock, "theme.css must declare @media (prefers-reduced-motion: reduce)");
  for (const prop of ["--dur-fast: 0ms", "--dur-med: 0ms", "animation-duration: 0.01ms !important", "animation-delay: 0ms !important", "transition-duration: 0.01ms !important", "transition-delay: 0ms !important", "scroll-behavior: auto !important"]) {
    assert.ok(rmBlock.includes(prop), "theme.css reduced-motion block missing: " + prop);
  }
  for (const file of ["index.html", "tabbar.html", "settings.html", "boot.html"]) {
    const pageHtml = readFileSync(new URL("../dist/" + file, import.meta.url), "utf8");
    assert.match(pageHtml, /<link\s+rel=["']stylesheet["']\s+href=["']theme\.css["']\s*\/?>/i, file + " must load theme.css");
    assert.doesNotMatch(pageHtml, /(?:animation|transition)[^;]*!important/i, file + " must not override reduced-motion with !important");
  }
});

test("stale-printer root-cause fix: single discovery cache invariant, force-refresh invalidation, and capsEpoch race guard", async () => {
  // 1. Verify Rust cache coherence invariant (no stacked outer printer_cache; DISCOVERY_TTL + 5s poll < 30s health TTL).
  const commandsRs = readFileSync(new URL("../src-tauri/src/commands.rs", import.meta.url), "utf8");
  const winRs = readFileSync(new URL("../../crates/print-win/src/win.rs", import.meta.url), "utf8");
  assert.equal(commandsRs.includes("pub printer_cache"), false, "duplicate outer printer_cache must be removed");
  assert.match(commandsRs, /print_win::invalidate_discovery_cache\(\)/, "force refresh must invalidate winspool caches");
  const ttlMatch = winRs.match(/const DISCOVERY_TTL:\s*std::time::Duration\s*=\s*std::time::Duration::from_secs\((\d+)\);/);
  assert.ok(ttlMatch, "DISCOVERY_TTL constant must exist");
  const discoveryTtl = Number(ttlMatch[1]);
  assert.ok(discoveryTtl + 5 < 30, `DISCOVERY_TTL (${discoveryTtl}s) + 5s UI poll must stay strictly below 30s health TTL`);

  // 2. Verify Control Center manualRefresh sends { force: true }, drops stale in-flight caps, and reloads settings.
  const { windowObj, document } = createMiniDom(html);
  const overviewCalls = [];
  let resolveStaleCaps;
  let capsCallCount = 0;
  let settingsCallCount = 0;
  const nowSec = Math.floor(Date.now() / 1000);
  const makePrinter = (port) => ({
    name: "POS-80",
    kind: "thermal",
    is_default: true,
    status: "ready",
    port,
    driver: "Generic / Text Only",
    protocol: "escpos",
    printer_class: "receipt",
    health: { source: "spooler", conditions: [], observed_at: nowSec - 17, ttl_secs: 30 },
  });

  windowObj.__NATIVE_PRINT__ = {
    async appOverview(opts) {
      overviewCalls.push(opts || null);
      return {
        appVersion: "1.5.7",
        os: "windows",
        odooOrigin: "https://erp.example.com",
        lockedByPolicy: false,
        autoReprint: true,
        pdfiumReady: true,
        webviewVersion: "120.0",
        environment: { es2022: true, localStorage: true, cookies: true, webSocket: true, secureContext: true, errors: [] },
        printers: [makePrinter(overviewCalls.length >= 2 ? "USB002" : "USB001")],
        defaultPrinter: "POS-80",
        stateCounts: [],
        recentJobs: [],
      };
    },
    async printerCapabilities() {
      capsCallCount++;
      if (capsCallCount === 1) {
        return new Promise((r) => { resolveStaleCaps = r; });
      }
      return { forms: [{ name: "80mm Fresh", size: { width_mm: 80, height_mm: 200 }, roll: true }], bins: ["Tray Fresh"] };
    },
    async getPrinterSettings() {
      settingsCallCount++;
      return { "POS-80": { paper: settingsCallCount >= 2 ? "80mm Fresh" : "80mm Old" } };
    },
  };

  const ctx = vm.createContext({
    ...windowObj,
    setInterval: windowObj.setInterval,
    setTimeout: windowObj.setTimeout,
    clearTimeout: windowObj.clearTimeout,
    clearInterval: windowObj.clearInterval,
    console,
    Date,
    Math,
    JSON,
    URL,
    Set,
    Map,
    Promise,
    Array,
    Object,
    Number,
    String,
    Boolean,
    RegExp,
    Error,
    encodeURIComponent,
    decodeURIComponent,
  });
  ctx.window = ctx;
  ctx.self = ctx;
  ctx.globalThis = ctx;
  new vm.Script(script).runInContext(ctx);
  await new Promise((r) => setTimeout(r, 15));

  const gridHtmlBefore = document.getElementById("printersGrid").innerHTML;
  assert.equal(gridHtmlBefore.includes("بيانات قديمة") || gridHtmlBefore.includes("Outdated"), false,
    "printer observed 17s ago (max age under 12s TTL + 5s poll) must never show stale badge");

  // Trigger manualRefresh while first printerCapabilities call is still in-flight.
  await ctx.manualRefresh();
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(overviewCalls[overviewCalls.length - 1]?.force, true,
    "manualRefresh must call appOverview({ force: true })");
  assert.ok(settingsCallCount >= 2, "manualRefresh must reload printerSettings");

  // Resolve the old in-flight caps call AFTER manualRefresh: epoch guard must ignore it.
  if (resolveStaleCaps) {
    resolveStaleCaps({ forms: [{ name: "STALE_FORM_OLD", size: { width_mm: 58, height_mm: 100 }, roll: true }], bins: ["STALE_TRAY"] });
  }
  await new Promise((r) => setTimeout(r, 15));
  const gridHtmlAfter = document.getElementById("printersGrid").innerHTML;
  assert.equal(gridHtmlAfter.includes("STALE_FORM_OLD"), false, "stale in-flight capabilities must be discarded by capsEpoch");
  assert.equal(gridHtmlAfter.includes("80mm Fresh"), true, "fresh capabilities must render after manualRefresh");
});

test("audit regression: double-submit guards, bridge-down validation, inspectForms i18n/epoch, and boot title sync", async () => {
  // 1. Control Center: double-submit guards + inspectForms empty-state i18n + bridge-down validateOriginField
  const { windowObj, document, MiniEl } = createMiniDom(html);
  let clearFinishedCalls = 0;
  let clearWebDataCalls = 0;
  let testPrintCalls = 0;
  let resolveClearFinished = null;
  let resolveTestPrint = null;
  let bridgeEnabled = true;

  windowObj.__NATIVE_PRINT__ = {
    async appOverview() {
      if (!bridgeEnabled) throw new Error("bridge_down");
      return {
        appVersion: "1.5.7",
        odooOrigin: "https://odoo.example.com",
        defaultPrinter: "HP OfficeJet",
        pdfiumReady: true,
        lockedByPolicy: false,
        autoReprint: false,
        stateCounts: [["done", 1]],
        printers: [{ name: "HP OfficeJet", kind: "physical", is_default: true, status: "ready", port: "USB001", driver: "HP", protocol: "gdi", printer_class: "office" }],
        recentJobs: [],
        environment: { es2022: true, localStorage: true, cookies: true, webSocket: true, secureContext: true, errors: [] },
      };
    },
    async printerCapabilities() { return { forms: [], bins: [], duplex: false, color: false }; },
    async getPrinterSettings() { return {}; },
    async printerForms() { return []; },
    async clearFinished() {
      clearFinishedCalls++;
      return new Promise((res) => { resolveClearFinished = res; });
    },
    async clearWebData() { clearWebDataCalls++; return true; },
    async runTestPrint() {
      testPrintCalls++;
      return new Promise((res) => { resolveTestPrint = res; });
    },
  };

  const ctx = vm.createContext({
    ...windowObj,
    setInterval: windowObj.setInterval,
    clearInterval: windowObj.clearInterval,
    setTimeout: windowObj.setTimeout,
    clearTimeout: windowObj.clearTimeout,
    localStorage: windowObj.localStorage,
    URL,
    Blob,
    Set,
    Map,
    Promise,
    Array,
    Object,
    Number,
    String,
    Boolean,
    RegExp,
    Error,
    encodeURIComponent,
    decodeURIComponent,
  });
  ctx.window = ctx;
  ctx.self = ctx;
  ctx.globalThis = ctx;
  new vm.Script(script).runInContext(ctx);
  await new Promise((r) => setTimeout(r, 15));

  // Double-submit guard on clearFinishedJobs
  const pClear1 = ctx.clearFinishedJobs();
  const pClear2 = ctx.clearFinishedJobs();
  assert.equal(document.getElementById("btnClearFinished").disabled, true);
  ctx.settleConfirm(true);
  await new Promise((r) => setImmediate(r));
  assert.equal(clearFinishedCalls, 1, "concurrent clearFinishedJobs must be blocked while in-flight");
  resolveClearFinished(1);
  await Promise.all([pClear1, pClear2]);
  assert.equal(document.getElementById("btnClearFinished").disabled, false);

  // Double-submit guard on runPrinterTest
  const pTest1 = ctx.runPrinterTest("HP OfficeJet", "pdf-a4");
  const pTest2 = ctx.runPrinterTest("HP OfficeJet", "pdf-a4");
  await new Promise((r) => setImmediate(r));
  assert.equal(testPrintCalls, 1, "concurrent runPrinterTest must be blocked while in-flight");
  resolveTestPrint("job-1");
  await Promise.all([pTest1, pTest2]);

  // inspectForms empty-state re-localizes when language switches
  const formsBox = new MiniEl("DIV", "forms-box-0");
  formsBox.classList.add("hidden");
  document.body.appendChild(formsBox);
  await ctx.inspectForms("HP OfficeJet", 0);
  assert.ok(formsBox.textContent.includes("لا توجد نماذج"), "Arabic empty forms message");
  ctx.toggleLang();
  assert.ok(document.getElementById("printersGrid").innerHTML.includes("No forms reported by driver."), "English empty forms message after language switch");

  // validateOriginField must keep save buttons disabled when native bridge is unavailable
  ctx.__NATIVE_PRINT__ = null;
  await ctx.refreshAll({ silent: true });
  document.getElementById("inputOdooOrigin").value = "https://odoo.example.com";
  ctx.validateOriginField();
  assert.equal(document.getElementById("btnSaveOrigin").disabled, true, "Save must remain disabled when bridge is unavailable");
  assert.equal(document.getElementById("btnSaveAndOpen").disabled, true, "Save & Open must remain disabled when bridge is unavailable");

  // 2. boot.html: document.title syncs with viewState and showInitFail clears count
  const bootHtml = readFileSync(new URL("../dist/boot.html", import.meta.url), "utf8");
  const bootScript = bootHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];
  const bootDom = createMiniDom(bootHtml);
  bootDom.windowObj.__TAURI_INTERNALS__ = {
    async invoke(cmd) {
      if (cmd === "app_identity") return { odooOrigin: "http://192.168.1.20:8069" };
      if (cmd === "probe_origin") return false;
    },
  };
  const bootCtx = vm.createContext({ ...bootDom.windowObj, setInterval: bootDom.windowObj.setInterval, clearInterval: bootDom.windowObj.clearInterval, setTimeout: bootDom.windowObj.setTimeout, clearTimeout: bootDom.windowObj.clearTimeout, localStorage: bootDom.windowObj.localStorage });
  new vm.Script(bootScript).runInContext(bootCtx);
  await new Promise((r) => setImmediate(r));
  await new Promise((r) => setImmediate(r));
  assert.equal(bootDom.document.title, "Yaseir Print — تعذّر الوصول إلى Odoo", "boot.html title must reflect down state");
  bootCtx.showInitFail();
  assert.equal(bootDom.document.getElementById("count").textContent, "", "showInitFail must clear countdown text");
  assert.equal(bootDom.document.title, "Yaseir Print — تعذّر بدء الاتصال", "boot.html title must reflect initFail state");
});

test("settings.html and tabbar.html audit regressions: re-entrancy locks, identity epoch, Escape drag cancel, and focus restore", async () => {
  // 1. settings.html: double-submit guards on action buttons + syncIdentity epoch race guard
  {
    const settingsHtml = readFileSync(new URL("../dist/settings.html", import.meta.url), "utf8");
    const settingsScript = settingsHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];
    const { windowObj, document } = createMiniDom(settingsHtml);
    const identityResolvers = [];
    let ccCalls = 0;
    let odooCalls = 0;
    let resolveCc = null;
    let resolveOdoo = null;

    windowObj.__TAURI_INTERNALS__ = {
      async invoke(cmd) {
        if (cmd === "app_identity") {
          return new Promise((res) => identityResolvers.push(res));
        }
        if (cmd === "open_control_center") {
          ccCalls++;
          return new Promise((res) => { resolveCc = res; });
        }
        if (cmd === "open_odoo_origin") {
          odooCalls++;
          return new Promise((res) => { resolveOdoo = res; });
        }
        return {};
      },
    };
    const ctx = vm.createContext({ ...windowObj, localStorage: windowObj.localStorage });
    new vm.Script(settingsScript).runInContext(ctx);
    assert.equal(identityResolvers.length, 1);

    // Trigger a newer visibilitychange syncIdentity() before the first resolves
    document.hidden = false;
    document.dispatch("visibilitychange");
    assert.equal(identityResolvers.length, 2);

    // Newer response resolves first with v2.0.0 and valid origin
    identityResolvers[1]({ appVersion: "2.0.0", odooOrigin: "https://odoo.example.com" });
    await new Promise((r) => setImmediate(r));
    assert.equal(document.getElementById("appVersion").textContent, "v2.0.0");
    assert.equal(document.getElementById("btnOpenOdoo").disabled, false);

    // Older stale response resolves second with v1.0.0 and no origin -> must be ignored
    identityResolvers[0]({ appVersion: "1.0.0", odooOrigin: "" });
    await new Promise((r) => setImmediate(r));
    assert.equal(document.getElementById("appVersion").textContent, "v2.0.0", "stale app_identity must not overwrite version");
    assert.equal(document.getElementById("btnOpenOdoo").disabled, false, "stale app_identity must not disable btnOpenOdoo");

    // Double-submit guard on btnOpenCc
    const btnCc = document.getElementById("btnOpenCc");
    btnCc.dispatch("click");
    btnCc.dispatch("click");
    assert.equal(ccCalls, 1, "concurrent btnOpenCc clicks must be blocked while in-flight");
    assert.equal(btnCc.disabled, true, "btnOpenCc must be disabled while in-flight");
    resolveCc({});
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setImmediate(r));
    assert.equal(btnCc.disabled, false, "btnOpenCc must re-enable after settlement");

    // Double-submit guard on btnOpenOdoo
    const btnOdoo = document.getElementById("btnOpenOdoo");
    btnOdoo.dispatch("click");
    btnOdoo.dispatch("click");
    assert.equal(odooCalls, 1, "concurrent btnOpenOdoo clicks must be blocked while in-flight");
    assert.equal(btnOdoo.disabled, true, "btnOpenOdoo must be disabled while in-flight");
    resolveOdoo({});
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setImmediate(r));
    assert.equal(btnOdoo.disabled, false, "btnOpenOdoo must restore enabled state when origin is configured");
  }

  // 2. tabbar.html: Escape cancels pointer drag, double-close/nav locks, and focus restore
  {
    const tabHtml = readFileSync(new URL("../dist/tabbar.html", import.meta.url), "utf8");
    const tabScript = tabHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];
    const { windowObj, document } = createMiniDom(tabHtml);
    let closeCalls = 0;
    let settingsCalls = 0;
    let moveCalls = 0;
    let resolveClose = null;
    let resolveSettings = null;
    let currentFixture = [
      { label: "t1", title: "Odoo", url: "https://odoo.example.com", active: true, is_pos: false },
      { label: "t2", title: "POS", url: "https://odoo.example.com/pos", active: false, is_pos: true },
    ];

    windowObj.__TAURI_INTERNALS__ = {
      async invoke(cmd) {
        if (cmd === "tab_list") return currentFixture;
        if (cmd === "tab_close") {
          closeCalls++;
          return new Promise((res) => { resolveClose = res; });
        }
        if (cmd === "open_settings") {
          settingsCalls++;
          return new Promise((res) => { resolveSettings = res; });
        }
        if (cmd === "tab_move") {
          moveCalls++;
          return {};
        }
        return {};
      },
    };
    const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, clearInterval: windowObj.clearInterval, localStorage: windowObj.localStorage });
    new vm.Script(tabScript).runInContext(ctx);
    await new Promise((r) => setImmediate(r));

    const strip = document.getElementById("tabs");
    const tabEls = strip.children.filter((c) => c.classList.contains("tab"));
    tabEls[0].getBoundingClientRect = () => ({ left: 100, width: 100 });
    tabEls[1].getBoundingClientRect = () => ({ left: 0, width: 100 });

    // Start pointer drag and press Escape -> must roll back without invoking tab_move
    tabEls[0].dispatch("pointerdown", { button: 0, clientX: 150, pointerId: 1 });
    windowObj.dispatch("pointermove", { clientX: 20 });
    assert.equal(tabEls[0].classList.contains("dragging"), true);
    windowObj.dispatch("keydown", { key: "Escape" });
    assert.equal(tabEls[0].classList.contains("dragging"), false, "Escape must clear dragging state");
    assert.equal(moveCalls, 0, "Escape must cancel drag without invoking tab_move");

    // Double-click close button on t2 -> only 1 tab_close in-flight
    const closeBtn = tabEls[1].children.find((c) => c.classList.contains("t-close"));
    closeBtn.dispatch("click");
    closeBtn.dispatch("click");
    assert.equal(closeCalls, 1, "concurrent closeTabWithHistory for same tab must be blocked");
    resolveClose({});
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setImmediate(r));

    // Double-click btnSettings -> only 1 open_settings in-flight
    const btnSettings = document.getElementById("btnSettings");
    btnSettings.dispatch("click");
    btnSettings.dispatch("click");
    assert.equal(settingsCalls, 1, "concurrent btnSettings clicks must be blocked while in-flight");
    resolveSettings({});
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setImmediate(r));

    // Focus restoration: focusing #btnNewTab survives a re-render
    document.getElementById("btnNewTab").focus();
    ctx.render([
      { label: "t1", title: "Odoo Updated", url: "https://odoo.example.com", active: true, is_pos: false },
      { label: "t2", title: "POS", url: "https://odoo.example.com/pos", active: false, is_pos: true },
    ]);
    assert.equal(document.activeElement.id, "btnNewTab", "render() must preserve focus on #btnNewTab");

    // Focus restoration: closing the focused tab falls back to the active tab
    const freshTabs = strip.children.filter((c) => c.classList.contains("tab"));
    freshTabs[1].focus();
    ctx.render([
      { label: "t1", title: "Odoo Updated", url: "https://odoo.example.com", active: true, is_pos: false },
    ]);
    assert.equal(document.activeElement.getAttribute("data-label"), "t1", "render() must fall back focus to active tab when focused tab is removed");
  }
});

test("boot.html targetUrl restores same-origin startupUrl and rejects off-origin", () => {
  const bootHtml = readFileSync(new URL("../dist/boot.html", import.meta.url), "utf8");
  const bootScript = bootHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];
  const targetSrc = bootScript.match(/function isHttpOrigin[\s\S]*?function tryProbe/)[0].replace(/function tryProbe$/, "");
  const fn = new vm.Script(
    "(function(){ var origin = null, startupUrl = null;\n" + targetSrc + "\nreturn function(o, s){ origin = o; startupUrl = s; return targetUrl(); }; })()"
  ).runInNewContext({ URL });
  assert.equal(fn("http://127.0.0.1:8069", "http://127.0.0.1:8069/pos/ui?config_id=1"), "http://127.0.0.1:8069/pos/ui?config_id=1");
  assert.equal(fn("http://127.0.0.1:8069", "https://evil.example.com/pos/ui"), "http://127.0.0.1:8069");
  assert.equal(fn("http://127.0.0.1:8069", null), "http://127.0.0.1:8069");
});

test("Odoo 19/20 qweb-text handler passes raw label modes directly via printText", () => {
  for (const ver of ["odoo17", "odoo18", "odoo19", "odoo20"]) {
    const h = readFileSync(new URL(`../../${ver}/native_print/static/src/js/native_print_handler.js`, import.meta.url), "utf8");
    assert.match(h, /\["escpos", "zpl", "tspl", "epl", "cpcl"\]\.includes\(rule\.mode\)/);
    assert.match(h, /mode:\s*rule\.mode,\s*text/);
  }
});

test("Calibration Lab: tcp:// selects thermal-80mm, populatePrinterSelect updates after user change, and IPv6 URI port parsing works", async () => {
  const { windowObj, document } = createMiniDom(html);
  const printers = [
    { name: "Office Laser", kind: "local", is_default: true, status: "ready", port: "192.168.1.50", driver: "HP" },
  ];
  windowObj.__NATIVE_PRINT__ = {
    async appOverview() {
      return { appVersion: "1.5.7", odooOrigin: "https://odoo.example.com", printers, defaultPrinter: "Office Laser", recentJobs: [], stateCounts: [] };
    },
    async printerCapabilities() { return { forms: [], duplex: false, color: false, bins: [] }; },
    async getPrinterSettings() { return {}; },
  };
  const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, clearInterval: windowObj.clearInterval, setTimeout: windowObj.setTimeout, clearTimeout: windowObj.clearTimeout, localStorage: windowObj.localStorage, navigator: {} });
  new vm.Script(script).runInContext(ctx);
  await new Promise(r => setImmediate(r));
  await new Promise(r => setImmediate(r));

  // (a) Selecting tcp:// sets customTestKind to thermal-80mm (not blank / pdf-a4)
  const printerSel = document.getElementById("customPrinterSelect");
  const kindSel = document.getElementById("customTestKind");
  printerSel.value = "tcp://192.168.1.50:9100";
  ctx.onTestPrinterChange();
  assert.equal(kindSel.value, "thermal-80mm", "tcp:// printer must default test kind to thermal-80mm");

  // (b) populatePrinterSelect still updates after user selection when new printer arrives, preserving current selection
  printerSel.dispatch("change");
  printers.push({ name: "Receipt Thermal", kind: "local", is_default: false, status: "ready", port: "USB001", driver: "Epson" });
  ctx.populatePrinterSelect(printers, "Office Laser");
  assert.ok(printerSel.innerHTML.includes("Receipt Thermal"), "new printer must appear in select after user previously changed it");
  assert.equal(printerSel.value, "tcp://192.168.1.50:9100", "previous selection must be preserved");

  // (c) IPv6 bracket-aware port parsing in allowedTestKinds() and validateCustomPrinterUriField()
  const uriInput = document.getElementById("customPrinterUri");
  printerSel.value = "__uri__";
  uriInput.value = "tcp://[fe80::631]:9100";
  assert.equal(ctx.validateCustomPrinterUriField(), true, "tcp://[fe80::631]:9100 must parse port 9100, not 631");
  assert.deepEqual([...ctx.allowedTestKinds()], ["thermal-80mm", "thermal-58mm", "zpl-label", "tspl-label", "epl-label"]);

  uriInput.value = "ipp://[fe80::9100]:631/ipp/print";
  assert.equal(ctx.validateCustomPrinterUriField(), true, "ipp://[fe80::9100]:631/ipp/print must parse port 631, not 9100");
  assert.deepEqual([...ctx.allowedTestKinds()], ["pdf-a4"]);

  uriInput.value = "tcp://[fe80::1]:631";
  assert.equal(ctx.validateCustomPrinterUriField(), false, "tcp://[fe80::1]:631 must be rejected (raw socket on IPP port 631)");
});

test("tabbar.html restoreSavedSession guards np_open_tabs, avoids duplicating /pos startupUrl, and restores active focus to tabs[0]", async () => {
  const tabHtml = readFileSync(new URL("../dist/tabbar.html", import.meta.url), "utf8");
  const tabScript = tabHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];
  const { windowObj } = createMiniDom(tabHtml);
  const posUrl = "https://odoo.example.com/pos/ui?config_id=1";
  const salesUrl = "https://odoo.example.com/web#menu_id=5";
  const savedTabs = [
    { label: "t1", title: "POS", url: posUrl, active: true, is_pos: true },
    { label: "t2", title: "Sales", url: salesUrl, active: false, is_pos: false },
  ];
  windowObj.localStorage.setItem("np_open_tabs", JSON.stringify(savedTabs));

  let resolveIdentity;
  const identityP = new Promise((r) => { resolveIdentity = r; });
  const liveTabs = [
    { label: "boot-0", title: "POS", url: posUrl, active: true, is_pos: true },
  ];
  const invoked = [];
  windowObj.__TAURI_INTERNALS__ = {
    async invoke(cmd, args) {
      invoked.push([cmd, args]);
      if (cmd === "app_identity") return identityP;
      if (cmd === "tab_open") {
        liveTabs.forEach((t) => { t.active = false; });
        liveTabs.push({ label: "tab-" + liveTabs.length, title: "Sales", url: args.url, active: true, is_pos: false });
        return {};
      }
      if (cmd === "tab_switch") {
        liveTabs.forEach((t) => { t.active = t.label === args.label; });
        return {};
      }
      if (cmd === "tab_list") return liveTabs.map((t) => ({ ...t }));
      return {};
    },
  };
  const ctx = vm.createContext({ ...windowObj, setInterval: windowObj.setInterval, clearInterval: windowObj.clearInterval, setTimeout: windowObj.setTimeout, clearTimeout: windowObj.clearTimeout, localStorage: windowObj.localStorage });
  new vm.Script(tabScript).runInContext(ctx);
  await new Promise((r) => setImmediate(r));

  // Initial refresh() ran while restoreSavedSession() is awaiting app_identity: np_open_tabs must NOT be overwritten with 1 tab
  const duringRestore = JSON.parse(windowObj.localStorage.getItem("np_open_tabs"));
  assert.equal(duringRestore.length, 2, "refresh() must not overwrite np_open_tabs while restoringSession is true");

  resolveIdentity({ odooOrigin: "https://odoo.example.com", startupUrl: posUrl });
  for (let i = 0; i < 6; i++) await new Promise((r) => setImmediate(r));

  const openedUrls = invoked.filter(([c]) => c === "tab_open").map(([, a]) => a.url);
  assert.deepEqual(openedUrls, [salesUrl], "must skip saved[0] when startupUrl matches /pos URL to avoid duplicate POS tab");
  assert.ok(invoked.some(([c, a]) => c === "tab_switch" && a.label === "boot-0"), "must switch focus back to tabs[0] when skipFirst && activeIdx === 0");
  const afterRestore = JSON.parse(windowObj.localStorage.getItem("np_open_tabs"));
  assert.equal(afterRestore.length, 2, "np_open_tabs must mirror restored tabs after restore completes");
});

test("np_tab_cmd same-millisecond monotonic id: sendTabCmd emits id+ts and handleCrossTabCommand never drops same-ms commands", async () => {
  // 1. Verify sendTabCmd emits ts + distinct monotonic id within the same millisecond
  const { windowObj: ccWin } = createMiniDom(html);
  const fixedNow = 1760000000000;
  const origNow = Date.now;
  Date.now = () => fixedNow;
  try {
    const ccCtx = vm.createContext({
      ...ccWin,
      Date,
      setInterval: ccWin.setInterval,
      clearInterval: ccWin.clearInterval,
      setTimeout: ccWin.setTimeout,
      clearTimeout: ccWin.clearTimeout,
      localStorage: ccWin.localStorage,
    });
    new vm.Script(script).runInContext(ccCtx);
    ccCtx.sendTabCmd({ action: "switch", label: "t1" });
    const cmd1 = JSON.parse(ccWin.localStorage.getItem("np_tab_cmd"));
    ccCtx.sendTabCmd({ action: "switch", label: "t2" });
    const cmd2 = JSON.parse(ccWin.localStorage.getItem("np_tab_cmd"));
    assert.equal(cmd1.ts, fixedNow, "cmd1 preserves ts");
    assert.equal(cmd2.ts, fixedNow, "cmd2 preserves ts");
    assert.ok(typeof cmd1.id === "string" && cmd1.id.startsWith(fixedNow + "-"), "cmd1 has monotonic id");
    assert.ok(typeof cmd2.id === "string" && cmd2.id.startsWith(fixedNow + "-"), "cmd2 has monotonic id");
    assert.notEqual(cmd1.id, cmd2.id, "same-ms commands must have distinct ids");

    // 2. Verify tabbar.html executes both same-ms commands and still dedupes identical id
    const tabHtml = readFileSync(new URL("../dist/tabbar.html", import.meta.url), "utf8");
    const tabScript = tabHtml.match(/<script\b[^>]*>([\s\S]*?)<\/script>/i)[1];
    const { windowObj: tbWin } = createMiniDom(tabHtml);
    const invoked = [];
    tbWin.__TAURI_INTERNALS__ = {
      async invoke(cmd, args) {
        invoked.push([cmd, args]);
        if (cmd === "tab_list") return [{ label: "t1", title: "A", url: "https://a.test", active: true }, { label: "t2", title: "B", url: "https://b.test", active: false }];
        return true;
      },
    };
    const tbCtx = vm.createContext({
      ...tbWin,
      Date,
      setInterval: tbWin.setInterval,
      clearInterval: tbWin.clearInterval,
      setTimeout: tbWin.setTimeout,
      clearTimeout: tbWin.clearTimeout,
      localStorage: tbWin.localStorage,
    });
    new vm.Script(tabScript).runInContext(tbCtx);

    tbWin.localStorage.setItem("np_tab_cmd", JSON.stringify(cmd1));
    tbCtx.handleCrossTabCommand();
    tbWin.localStorage.setItem("np_tab_cmd", JSON.stringify(cmd2));
    tbCtx.handleCrossTabCommand();
    // Replay cmd2 with identical id -> must be deduped
    tbWin.localStorage.setItem("np_tab_cmd", JSON.stringify(cmd2));
    tbCtx.handleCrossTabCommand();

    const switches = invoked.filter(([c]) => c === "tab_switch").map(([, a]) => a.label);
    assert.deepEqual(switches, ["t1", "t2"], "both same-ms commands execute and duplicate id is deduped");
  } finally {
    Date.now = origNow;
  }
});

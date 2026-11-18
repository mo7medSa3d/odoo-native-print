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
    "http://8.8.8.8:8069", "http://68.221.27.248:8070",
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
    for (const token of ["--text-primary", "--text-secondary", "--text-muted", "--brand-text", "--accent-cyan"]) {
      const fg = parseColor(vars[token]);
      for (const [bgName, bg] of [["--bg-surface", surface], ["--bg-canvas", canvas], ["--bg-subtle", subtle]]) {
        const cr = ratio(fg, bg);
        assert.ok(cr >= 4.5, `${name} ${token} on ${bgName} contrast ${cr.toFixed(2)}:1 < 4.5:1`);
      }
    }

    // Brand button text >= 4.5:1 on brand & brand-hover
    for (const bgToken of ["--brand", "--brand-hover"]) {
      const cr = ratio(parseColor(vars["--brand-fg"]), parseColor(vars[bgToken]));
      assert.ok(cr >= 4.5, `${name} --brand-fg on ${bgToken} contrast ${cr.toFixed(2)}:1 < 4.5:1`);
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

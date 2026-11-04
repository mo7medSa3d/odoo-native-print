import { JSDOM } from "jsdom";
import fs from "node:fs";
import { extractBridge } from "./extract-bridge.mjs";
const bridge = extractBridge();
let pass = 0, fail = 0;
const ok = (name, cond) => { cond ? pass++ : fail++; console.log((cond ? "PASS " : "FAIL ") + name); };
const BOOTED = '<div class="o_web_client"><div class="o_action_manager"><div class="o_list_view">rows</div></div></div>';
function makePage(path, bodyHtml, url) {
  const dom = new JSDOM(`<!DOCTYPE html><html><body>${bodyHtml}</body></html>`, {
    url: url || ("http://68.221.27.248:8070" + path), runScripts: "outside-only", pretendToBeVisual: true,
  });
  const w = dom.window;
  w.__TAURI_INTERNALS__ = { invoke: async () => ({}) };
  w.__TAURI__ = { core: { invoke: async () => ({}) } };
  w.__NP_BOOT_DELAYS__ = { tickMs: 20, failAfter: 5 };
  w.eval(bridge);
  return { dom, w };
}
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
// A: healthy boot (shell + action content) -> no overlay, nav pill present
{
  const { w } = makePage("/odoo", BOOTED);
  await sleep(250);
  ok("healthy: no overlay", !w.document.getElementById("np-boot-overlay"));
  const pill = w.document.getElementById("np-nav-pill");
  ok("healthy: nav pill with 3 buttons", !!pill && pill.querySelectorAll("button").length === 3);
  w.close();
}
// B: slow boot -> corner card appears, then auto-hides when shell+content arrive
{
  const { w } = makePage("/odoo", '<div id="app"></div>');
  await sleep(180);
  const el = w.document.getElementById("np-boot-overlay");
  ok("slow: card appears after deadline", !!el);
  ok("slow: card is NOT a full-screen cover", !!el && !el.getAttribute("style").includes("inset:0") && el.getAttribute("style").includes("bottom:16px"));
  const shell = w.document.createElement("div"); shell.innerHTML = BOOTED;
  w.document.body.appendChild(shell.firstChild);
  await sleep(150);
  ok("slow: card auto-hides on late boot", !w.document.getElementById("np-boot-overlay"));
  w.close();
}
// B2: shell mounts but content never renders -> card after the content grace window
{
  const { w } = makePage("/odoo", '<div class="o_web_client"></div>');
  await sleep(750); // shell at tick ~1, content grace = 20 ticks * 20ms
  ok("shell-only: card appears", !!w.document.getElementById("np-boot-overlay"));
  w.close();
}
// C: never boots -> card stays, buttons wired
{
  const { w } = makePage("/odoo", '<div id="app"></div>');
  await sleep(180);
  const el = w.document.getElementById("np-boot-overlay");
  ok("dead: card present", !!el);
  ok("dead: 3 action buttons", !!w.document.getElementById("np-boot-reload") && !!w.document.getElementById("np-boot-clear") && !!w.document.getElementById("np-boot-cc"));
  w.close();
}
// D: login page -> watchdog silent, no nav pill assertions needed
{
  const { w } = makePage("/web/login", '<form class="oe_login_form"></form>');
  await sleep(200);
  ok("login: no overlay", !w.document.getElementById("np-boot-overlay"));
  w.close();
}
// D2: backend page soft-redirects to login (session expiry) -> no card over login
{
  const { w } = makePage("/odoo", '<div id="app"></div>');
  await sleep(60);
  w.history.pushState({}, "", "/web/login");
  await sleep(250);
  ok("expiry redirect: no overlay over login", !w.document.getElementById("np-boot-overlay"));
  w.close();
}
// E: translations rejection -> card explains the server-side cause in Arabic
{
  const { w } = makePage("/odoo", '<div id="app"></div>');
  const ev = new w.Event("unhandledrejection");
  ev.reason = new Error("Error while fetching translations");
  w.dispatchEvent(ev);
  await sleep(220);
  const el = w.document.getElementById("np-boot-overlay");
  ok("translations: card present", !!el);
  ok("translations: card names the server cause", !!el && el.textContent.includes("ملف الترجمة") && el.textContent.includes("native_print"));
  w.close();
}
// F: local Control Center page -> no facade-driven watchdog/pill
{
  const { w } = makePage("/index.html", '<div></div>', "tauri://localhost/index.html");
  await sleep(150);
  ok("local: no overlay", !w.document.getElementById("np-boot-overlay"));
  ok("local: no nav pill", !w.document.getElementById("np-nav-pill"));
  w.close();
}
console.log(`watchdog 1.4.7 scenarios: ${pass} pass, ${fail} fail`);
process.exit(fail ? 1 : 0);

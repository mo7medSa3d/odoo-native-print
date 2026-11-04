import { JSDOM } from "jsdom";
import fs from "node:fs";
import { extractBridge } from "./extract-bridge.mjs";
const bridge = extractBridge();
let pass = 0, fail = 0;
const ok = (name, cond) => { cond ? pass++ : fail++; console.log((cond ? "PASS " : "FAIL ") + name); };
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
function makePage(path) {
  const dom = new JSDOM(`<!DOCTYPE html><html><body><div id="app"></div></body></html>`, {
    url: "http://68.221.27.248:8070" + path, runScripts: "dangerously", pretendToBeVisual: true,
  });
  const w = dom.window;
  const reports = [];
  w.__TAURI_INTERNALS__ = { invoke: async (cmd, args) => { if (cmd === "report_environment") reports.push(JSON.parse(args.report)); return {}; } };
  w.__NP_BOOT_DELAYS__ = { tickMs: 20, failAfter: 5 };
  const s = w.document.createElement("script"); s.textContent = bridge; w.document.body.appendChild(s);
  return { dom, w, reports };
}
function runPageScript(w, code) {
  const s = w.document.createElement("script"); s.textContent = code; w.document.body.appendChild(s);
}
// A: a real page script that throws is captured, reported, and shown on the card
{
  const { w, reports } = makePage("/odoo");
  runPageScript(w, `throw new TypeError("Cannot add property x, object is not extensible");`);
  await sleep(700);
  ok("tap: page TypeError captured", (w.__NP_ERRORS__ || []).some(e => e.includes("not extensible")));
  ok("tap: error included in environment report", reports.some(r => Array.isArray(r.errors) && r.errors.some(e => e.includes("not extensible"))));
  const card = w.document.getElementById("np-boot-overlay");
  ok("card: appears and lists the page error", !!card && card.textContent.includes("not extensible"));
  w.close();
}
// B: unhandled rejection captured
{
  const { w } = makePage("/odoo");
  const ev = new w.Event("unhandledrejection");
  ev.reason = new Error("bundle init failed");
  w.dispatchEvent(ev);
  await sleep(60);
  ok("tap: unhandled rejection captured", (w.__NP_ERRORS__ || []).some(e => e.includes("bundle init failed")));
  w.close();
}
// C: failed asset load captured (capture-phase error on a script element)
{
  const { w } = makePage("/odoo");
  const s = w.document.createElement("script");
  s.src = "http://68.221.27.248:8070/web/assets/bundle.js";
  w.document.body.appendChild(s);
  s.dispatchEvent(new w.Event("error"));
  await sleep(60);
  ok("tap: asset load failure captured", (w.__NP_ERRORS__ || []).some(e => e.startsWith("load:")));
  w.close();
}
// D: clean login page -> empty errors, no overlay
{
  const { w, reports } = makePage("/web/login");
  await sleep(250);
  ok("clean page: errors array empty in report", reports.length === 0 || (reports[0] && Array.isArray(reports[0].errors) && reports[0].errors.length === 0));
  ok("clean page: no overlay on login", !w.document.getElementById("np-boot-overlay"));
  w.close();
}
console.log(`error tap scenarios: ${pass} pass, ${fail} fail`);
process.exit(fail ? 1 : 0);

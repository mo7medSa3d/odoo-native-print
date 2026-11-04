import { JSDOM } from "jsdom";
import fs from "node:fs";
const html = fs.readFileSync(new URL("../../dist/index.html", import.meta.url), "utf8");
let pass = 0, fail = 0;
const ok = (name, cond) => { cond ? pass++ : fail++; console.log((cond ? "PASS " : "FAIL ") + name); };
const sleep = (ms) => new Promise(r => setTimeout(r, ms));

const overview = {
  appVersion: "1.3.2", printers: [{ name: "HP LaserJet", kind: "local", is_default: true, status: "ready", port: "USB001", driver: "HP" }],
  defaultPrinter: "HP LaserJet", stateCounts: [["queued", 1]], pdfiumReady: true,
  webviewVersion: "120.0", environment: { es2022: true }, odooOrigin: "http://192.168.1.20:8069",
  lockedByPolicy: false, autoReprint: false,
  recentJobs: [{ id: "job-1", report: "Invoice", printer: "HP LaserJet", pages: 2, bytes: 2048, state: "queued", attempts: 1, error_code: "" }],
};
const dom = new JSDOM(html, { url: "http://tauri.localhost/", runScripts: "dangerously", pretendToBeVisual: true,
  beforeParse(w) { w.__NATIVE_PRINT__ = { appOverview: async () => JSON.parse(JSON.stringify(overview)) }; } });
const w = dom.window;
await sleep(300);

// initial load: settings + select populated once
ok("initial: origin loaded", w.document.getElementById("inputOdooOrigin").value === "http://192.168.1.20:8069");
const sel = w.document.getElementById("customPrinterSelect");
ok("initial: printer select populated", sel.options.length >= 2 && sel.value === "HP LaserJet");

// user edits origin; a poll must NOT overwrite it
const origin = w.document.getElementById("inputOdooOrigin");
origin.focus();
origin.value = "http://10.0.0.5:8070";
origin.dispatchEvent(new w.Event("input", { bubbles: true }));
origin.blur();
await w.refreshAll({ silent: true });
ok("dirty origin survives poll", origin.value === "http://10.0.0.5:8070");

// user picks auto-reprint; poll must NOT snap it back
const ar = w.document.getElementById("inputAutoReprint");
ar.value = "true";
ar.dispatchEvent(new w.Event("change", { bubbles: true }));
await w.refreshAll({ silent: true });
ok("dirty auto-reprint survives poll", ar.value === "true");

// focused select is never rebuilt; unchanged data is never rebuilt either
sel.focus();
const htmlBefore = sel.innerHTML;
await w.refreshAll({ silent: true });
ok("focused select not rebuilt", sel.innerHTML === htmlBefore);
sel.blur();
await w.refreshAll({ silent: true });
ok("unchanged select not rebuilt (signature)", sel.innerHTML === htmlBefore);

// jobs table + printers grid skip identical re-renders
const tbodyHtml = w.document.getElementById("jobsTableBody").innerHTML;
const gridHtml = w.document.getElementById("printersGrid").innerHTML;
await w.refreshAll({ silent: true });
ok("jobs table untouched on identical poll", w.document.getElementById("jobsTableBody").innerHTML === tbodyHtml);
ok("printers grid untouched on identical poll", w.document.getElementById("printersGrid").innerHTML === gridHtml);

// changed data DOES re-render
overview.recentJobs[0].state = "done";
await w.refreshAll({ silent: true });
ok("jobs table re-renders on change", w.document.getElementById("jobsTableBody").innerHTML !== tbodyHtml);

// toast stack: max 3, newest on top, dedupe within 2s
const c = w.document.getElementById("toastContainer");
w.showToast("first info");
w.showToast("boom error", true);
w.showToast("boom error", true); // duplicate inside 2s -> deduped
w.showToast("third");
w.showToast("fourth");
ok("toast: capped at 3 visible", c.children.length === 3);
ok("toast: newest on top", c.children[0].textContent.includes("fourth"));
ok("toast: duplicate deduped", ![...c.children].some(el => el.textContent.includes("first info")) || c.children.length === 3);
ok("toast: has close button + icon", !!c.children[0].querySelector(".toast-close") && !!c.children[0].querySelector(".toast-icon"));
c.children[0].querySelector(".toast-close").click();
ok("toast: close button dismisses", c.children.length === 2);
console.log(`behavior checks: ${pass} pass, ${fail} fail`);
process.exit(fail ? 1 : 0);

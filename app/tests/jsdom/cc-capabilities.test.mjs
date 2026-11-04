// Capability truth on the Control Center printer cards and the
// calibration lab (owner, 2026-11-04): a card offers only the test
// prints its printer can actually accept, shows the printer's real
// class, and displays only driver-reported capabilities; the lab's
// calibration dropdown follows the selected printer's protocol.
import { JSDOM } from "jsdom";
import fs from "node:fs";
const html = fs.readFileSync(new URL("../../dist/index.html", import.meta.url), "utf8");
let pass = 0, fail = 0;
const ok = (name, cond) => { cond ? pass++ : fail++; console.log((cond ? "PASS " : "FAIL ") + name); };
const sleep = (ms) => new Promise(r => setTimeout(r, ms));

const printers = [
  { name: "OneNote (Desktop)", kind: "virtual", protocol: "gdi", printer_class: "virtual", is_default: false, status: "ready", port: "nul:", driver: "Send to Microsoft OneNote 16 Driver" },
  { name: "EPSON TM-T88VII", kind: "network", protocol: "escpos", printer_class: "thermal", is_default: false, status: "ready", port: "192.168.1.50", driver: "EPSON" },
  { name: "Zebra ZD421", kind: "local", protocol: "zpl", printer_class: "label", is_default: false, status: "ready", port: "USB002", driver: "Zebra" },
  { name: "HP LaserJet", kind: "local", protocol: "gdi", printer_class: "office", is_default: true, status: "ready", port: "USB001", driver: "HP PCL" },
];
const capsByName = {
  "HP LaserJet": { forms: [{ name: "A4", size: { width_mm: 210, height_mm: 297 }, roll: false }, { name: "Letter", size: { width_mm: 216, height_mm: 279 }, roll: false }], resolutions_dpi: [600], duplex: true, color: false },
  "EPSON TM-T88VII": { forms: [{ name: "Roll 80mm", size: { width_mm: 80, height_mm: 220 }, roll: true }], resolutions_dpi: [203], duplex: false, color: false },
  "Zebra ZD421": { forms: [{ name: "100x150", size: { width_mm: 100, height_mm: 150 }, roll: true }], resolutions_dpi: [203], duplex: false, color: false },
  "OneNote (Desktop)": { forms: [{ name: "A4", size: { width_mm: 210, height_mm: 297 }, roll: false }], resolutions_dpi: [], duplex: false, color: true },
};
const overview = {
  appVersion: "1.4.11", printers, defaultPrinter: "HP LaserJet",
  stateCounts: [], pdfiumReady: true, webviewVersion: "120.0",
  environment: { es2022: true }, odooOrigin: "http://192.168.1.20:8069",
  lockedByPolicy: false, autoReprint: false, recentJobs: [],
};
const dom = new JSDOM(html, { url: "http://tauri.localhost/", runScripts: "dangerously", pretendToBeVisual: true,
  beforeParse(w) {
    w.__NATIVE_PRINT__ = {
      appOverview: async () => JSON.parse(JSON.stringify(overview)),
      printerCapabilities: async (name) => JSON.parse(JSON.stringify(capsByName[name] || {})),
    };
  } });
const w = dom.window;
await sleep(500);

const cards = [...w.document.querySelectorAll(".printer-card")];
ok("grid: 4 cards rendered", cards.length === 4);
const cardOf = (name) => cards.find((c) => c.textContent.includes(name));
const kindsOf = (card) => [...card.querySelectorAll("[data-action='test-print']")].map((b) => b.getAttribute("data-kind"));

// OneNote (virtual GDI): PDF only — no raw buttons may exist.
{
  const c = cardOf("OneNote (Desktop)");
  ok("onenote: only the PDF quick test is offered", JSON.stringify(kindsOf(c)) === JSON.stringify(["pdf-a4"]));
  ok("onenote: class badge says Virtual GDI (not NETWORK)", c.textContent.includes("GDI") && !c.textContent.includes("NETWORK"));
  ok("onenote: no duplex chip (driver does not report it)", !c.textContent.includes("Duplex") && !c.textContent.includes("وجهان"));
}
// EPSON thermal: ESC/POS only.
{
  const c = cardOf("EPSON TM-T88VII");
  ok("epson: pdf + thermal offered, no zpl", JSON.stringify(kindsOf(c)) === JSON.stringify(["pdf-a4", "thermal-80mm"]));
  ok("epson: roll form chip shown from driver caps", c.textContent.includes("Roll 80mm"));
}
// Zebra label: ZPL only.
{
  const c = cardOf("Zebra ZD421");
  ok("zebra: pdf + zpl offered, no thermal", JSON.stringify(kindsOf(c)) === JSON.stringify(["pdf-a4", "zpl-label"]));
}
// HP office: PDF only, duplex chip from driver caps.
{
  const c = cardOf("HP LaserJet");
  ok("hp: only the PDF quick test is offered", JSON.stringify(kindsOf(c)) === JSON.stringify(["pdf-a4"]));
  ok("hp: duplex chip shown (driver reports duplex)", c.textContent.includes("Duplex") || c.textContent.includes("وجهان"));
  ok("hp: A4 form chip shown", c.textContent.includes("A4"));
}
// The calibration lab follows the SELECTED printer's protocol.
const sel = w.document.getElementById("customPrinterSelect");
const kindSel = w.document.getElementById("customTestKind");
const kindValues = () => [...kindSel.options].map((o) => o.value);
sel.value = "EPSON TM-T88VII";
w.onTestPrinterChange();
ok("lab: thermal printer -> pdf + both thermal kinds",
  JSON.stringify(kindValues()) === JSON.stringify(["pdf-a4", "thermal-80mm", "thermal-58mm"]));
sel.value = "OneNote (Desktop)";
w.onTestPrinterChange();
ok("lab: virtual printer -> pdf only", JSON.stringify(kindValues()) === JSON.stringify(["pdf-a4"]));
sel.value = "Zebra ZD421";
w.onTestPrinterChange();
ok("lab: label printer -> pdf + zpl", JSON.stringify(kindValues()) === JSON.stringify(["pdf-a4", "zpl-label"]));
sel.value = "HP LaserJet";
w.onTestPrinterChange();
ok("lab: office printer -> pdf only", JSON.stringify(kindValues()) === JSON.stringify(["pdf-a4"]));

console.log(`\nCC CAPABILITIES: ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);

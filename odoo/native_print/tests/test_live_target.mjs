// Node tests for the live printer helpers in native_print_client.js.
// Run: node odoo/native_print/tests/test_live_target.mjs
import { readFileSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";

const src = readFileSync(
    new URL("../static/src/js/native_print_client.js", import.meta.url),
    "utf8",
).replace(
    // Stub the Odoo translation import for the bare-node harness.
    'import { _t } from "@web/core/l10n/translation";',
    "const _t = (s) => s;",
);
const tmp = join(tmpdir(), "np_client_test.mjs");
writeFileSync(tmp, src);
const m = await import(tmp);

const eq = (a, b, msg) => {
    if (a !== b) {
        console.error("FAIL", msg, a, b);
        process.exit(1);
    }
};

eq(m.detectPrinterType({ name: "EPSON TM-T88VII", driver: "EPSON TM-T88VII", kind: "network" }).mode, "escpos", "epson mode");
eq(m.detectPrinterType({ name: "EPSON TM-T88VII" }).printerClass, "thermal", "epson class");
eq(m.detectPrinterType({ name: "ZDesigner ZT230", driver: "ZDesigner" }).mode, "zpl", "zebra");
eq(m.detectPrinterType({ name: "HP LaserJet", driver: "HP PCL" }).mode, "gdi", "hp");
eq(m.detectPrinterType({ name: "Microsoft Print to PDF", kind: "virtual" }).printerClass, "virtual", "virtual");
eq(m.printerPatternFor({ name: "EPSON TM-T88VII", port: "192.168.1.50" }, "escpos"), "tcp://192.168.1.50:9100", "tcp pattern");
eq(m.printerPatternFor({ name: "HP LaserJet", port: "192.168.1.51" }, "gdi"), "HP LaserJet", "spooler pattern");
// Audit L9/L10 hardening: IPv6 ports are bracketed, invalid IPv4
// octets are not IPs, and protocol tokens match on word boundaries.
eq(m.printerPatternFor({ name: "Label", port: "::1" }, "zpl"), "tcp://[::1]:9100", "ipv6 bracketed");
eq(m.printerPatternFor({ name: "Label", port: "999.1.1.1" }, "zpl"), "Label", "bad octet not an ip");
eq(m.detectPrinterType({ name: "Replit OfficeJet", driver: "HP" }).mode, "gdi", "epl substring is not epl");
eq(m.detectPrinterType({ name: "Zebra ZPL Label", driver: "Zebra" }).mode, "zpl", "zpl word");
eq(m.detectPrinterType({ name: "TSC TTP-244", driver: "TSC" }).mode, "tspl", "tsc word");
eq(m.rollWidthMm({ forms: [{ roll: true, size: { width_mm: 80 } }] }), 80, "roll width");
eq(m.rollWidthMm({ forms: [{ roll: false, size: { width_mm: 210 } }] }), null, "no roll");
// Bridge-shipped detection wins over the name heuristic: the app
// is the single detection authority (field bug, 2026-11-05 — the
// two detectors disagreed and valid POS printers were refused).
eq(m.detectPrinterType({ name: "POS Printer", driver: "Generic", protocol: "escpos", printer_class: "thermal" }).mode, "escpos", "bridge protocol wins");
eq(m.detectPrinterType({ name: "EPSON TM-T88VII", driver: "EPSON", protocol: "gdi", printer_class: "office" }).mode, "gdi", "bridge protocol wins over heuristic");
// Generic clone-driver identities are thermal in the fallback
// heuristic too (mirrors the Rust token table).
eq(m.detectPrinterType({ name: "POS Printer", driver: "POS Printer Driver" }).mode, "escpos", "generic pos printer");
eq(m.detectPrinterType({ name: "Thermal Printer", driver: "Generic" }).printerClass, "thermal", "generic thermal printer");
eq(m.detectPrinterType({ name: "POS80", driver: "POS Printer Driver" }).mode, "escpos", "pos80");
eq(m.detectPrinterType({ name: "PostScript Printer", driver: "Adobe PDF" }).mode, "gdi", "postscript is not pos");

globalThis.window = {
    localStorage: {
        _d: {},
        getItem(k) { return this._d[k] || null; },
        setItem(k, v) { this._d[k] = v; },
        removeItem(k) { delete this._d[k]; },
    },
};
const printers = [
    { name: "HP LaserJet", driver: "HP PCL", kind: "local", is_default: true },
    { name: "EPSON TM-T88VII", driver: "EPSON", kind: "network", port: "192.168.1.50", is_default: false },
];
const bridge = {
    listPrinters: async () => printers,
    printerCapabilities: async () => ({ forms: [{ roll: true, size: { width_mm: 80 } }], duplex: false, color: true }),
};
let t = await m.resolveBridgeTarget(bridge);
eq(t.printer.name, "HP LaserJet", "default first");
m.setSelectedPrinter("EPSON TM-T88VII");
t = await m.resolveBridgeTarget(bridge);
eq(t.printer.name, "EPSON TM-T88VII", "selected wins");
eq(t.rule.printer_pattern, "tcp://192.168.1.50:9100", "selected tcp");
eq(t.rule.media.roll_width_mm, 80, "selected roll");
t = await m.resolveBridgeTarget(bridge, { preferClasses: ["thermal", "label"] });
eq(t.printer.name, "EPSON TM-T88VII", "pos prefers thermal");
// Owner's selection model (2026-11-04): an explicit panel
// selection wins even over the POS class preference — the owner
// picks the printer in the panel and everything goes there.
m.setSelectedPrinter("HP LaserJet");
t = await m.resolveBridgeTarget(bridge, { preferClasses: ["thermal", "label"] });
eq(t.printer.name, "HP LaserJet", "selected office wins over pos class preference");
m.setSelectedPrinter(null);
t = await m.resolveBridgeTarget(bridge, { preferClasses: ["thermal", "label"] });
eq(t.printer.name, "EPSON TM-T88VII", "no selection -> class preference applies");
m.setSelectedPrinter("Gone Printer");
t = await m.resolveBridgeTarget(bridge);
eq(t.printer.name, "HP LaserJet", "stale selection falls back to default");
t = await m.resolveBridgeTarget({ listPrinters: async () => [] });
eq(t, null, "no printers -> null");
console.log("JS LIVE TARGET TESTS: ALL PASS");

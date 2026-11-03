// Node tests for the live printer helpers in native_print_client.js.
// Run: node odoo/native_print/tests/test_live_target.mjs
import { readFileSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";

const src = readFileSync(
    new URL("../static/src/js/native_print_client.js", import.meta.url),
    "utf8",
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
eq(m.rollWidthMm({ forms: [{ roll: true, size: { width_mm: 80 } }] }), 80, "roll width");
eq(m.rollWidthMm({ forms: [{ roll: false, size: { width_mm: 210 } }] }), null, "no roll");

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
m.setSelectedPrinter("Gone Printer");
t = await m.resolveBridgeTarget(bridge);
eq(t.printer.name, "HP LaserJet", "stale selection falls back to default");
t = await m.resolveBridgeTarget({ listPrinters: async () => [] });
eq(t, null, "no printers -> null");
console.log("JS LIVE TARGET TESTS: ALL PASS");

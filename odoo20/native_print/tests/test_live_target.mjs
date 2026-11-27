// Node tests for the live printer helpers in native_print_client.js.
// Run: node --test odoo20/native_print/tests/test_live_target.mjs
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { pathToFileURL } from "node:url";
import vm from "node:vm";

const src = readFileSync(
    new URL("../static/src/js/native_print_client.js", import.meta.url),
    "utf8",
).replace(
    // Stub the Odoo translation import for the bare-node harness.
    'import { _t } from "@web/core/l10n/translation";',
    "const _t = (s) => s;",
);
const tmpDir = mkdtempSync(join(tmpdir(), "np-live-target-"));
process.on("exit", () => { try { rmSync(tmpDir, { recursive: true, force: true }); } catch {} });
const tmp = join(tmpDir, "np_client_test.mjs");
writeFileSync(tmp, src);
const tmpUrl = pathToFileURL(tmp).href;
const m = await import(tmpUrl);

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
eq(m.detectPrinterType({ name: "Microsoft Print to PDF", kind: "virtual", protocol: "escpos", printer_class: "thermal" }).mode, "gdi", "virtual overrides conflicting raw protocol");
eq(m.detectPrinterType({ name: "Microsoft XPS Document Writer", kind: "virtual", protocol: "zpl", printer_class: "label" }).printerClass, "virtual", "virtual overrides conflicting label class");
eq(m.printerPatternFor({ name: "EPSON TM-T88VII", port: "192.168.1.50" }, "escpos"), "tcp://192.168.1.50:9100", "tcp pattern");
eq(m.printerPatternFor({ name: "EPSON TM-T88VII", port: "IP_192.168.1.50" }, "escpos"), "tcp://192.168.1.50:9100", "windows IP_ tcp pattern");
eq(m.printerPatternFor({ name: "EPSON TM-T88VII", port: "127.0.0.1:9100" }, "escpos"), "tcp://127.0.0.1:9100", "host:port tcp pattern");
eq(m.printerPatternFor({ name: "HP LaserJet", port: "192.168.1.51" }, "gdi"), "HP LaserJet", "spooler pattern");
// IP/protocol hardening: IPv6 ports are bracketed, invalid IPv4
// octets are not IPs, and protocol tokens match on word boundaries.
eq(m.printerPatternFor({ name: "Label", port: "::1" }, "zpl"), "tcp://[::1]:9100", "ipv6 bracketed");
eq(m.printerPatternFor({ name: "Label", port: "999.1.1.1" }, "zpl"), "Label", "bad octet not an ip");
eq(m.detectPrinterType({ name: "Replit OfficeJet", driver: "HP" }).mode, "gdi", "epl substring is not epl");
eq(m.detectPrinterType({ name: "Zebra ZPL Label", driver: "Zebra" }).mode, "zpl", "zpl word");
eq(m.detectPrinterType({ name: "TSC TTP-244", driver: "TSC" }).mode, "tspl", "tsc word");
eq(m.detectPrinterType({ name: "Mobile CPCL Label", driver: "Generic" }).mode, "cpcl", "cpcl word");
eq(m.detectPrinterType({ name: "Mobile CPCL Label", driver: "Generic" }).printerClass, "label", "cpcl class");
eq(m.detectPrinterType({ name: "MCPCL Office", driver: "HP" }).mode, "gdi", "cpcl substring is not cpcl");
eq(m.printerPatternFor({ name: "Mobile CPCL", port: "192.168.1.60" }, "cpcl"), "tcp://192.168.1.60:9100", "cpcl tcp pattern");
// Wider POS ecosystem: the JS fallback table
// carries the same identities as the Rust detection, so a real
// POS printer is recognized even with an old bridge.
eq(m.detectPrinterType({ name: "XP-80C", driver: "USB Printing Support" }).mode, "escpos", "xprinter model name");
eq(m.detectPrinterType({ name: "BTP-R880NP", driver: "SNBC" }).mode, "escpos", "snbc btp model");
eq(m.detectPrinterType({ name: "80mm Mini Printer", driver: "Generic USB" }).printerClass, "thermal", "width-only identity");
eq(m.detectPrinterType({ name: "小票打印机", driver: "热敏驱动" }).mode, "escpos", "chinese clone driver");
eq(m.rollWidthMm({ forms: [{ roll: true, size: { width_mm: 80 } }] }), 80, "roll width");
eq(m.rollWidthMm({ forms: [{ roll: false, size: { width_mm: 210 } }] }), null, "no roll");
eq(m.rollWidthMm({ forms: [{ roll: true, width_mm: 58 }] }), 58, "flat roll width tolerated");
eq(m.rollWidthMm({ forms: [{ roll: true, size: { width_mm: 0 } }] }), null, "zero roll width ignored");
eq(m.printerPatternFor(null, "gdi"), "", "null printer returns empty pattern");
// Bridge-shipped detection wins over the name heuristic: the app
// is the single detection authority (the
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
// Selection precedence: an explicit panel
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
// Stale empty printer cache must be bypassed once with { force: true }.
const forceCalls = [];
t = await m.resolveBridgeTarget({
    listPrinters: async (opts) => {
        forceCalls.push(opts || null);
        return opts && opts.force ? [printers[0]] : [];
    },
    printerCapabilities: async () => ({ forms: [], color: true }),
});
eq(t && t.printer.name, "HP LaserJet", "empty cached list retries with force: true");
eq(Boolean(forceCalls[1] && forceCalls[1].force), true, "second listPrinters call passes force: true");

// Duplex is a choice, not a capability: even a
// duplex-CAPABLE printer's rule must leave duplex unset, so the
// driver default (or a saved per-printer default in the app)
// decides — never the capability probe.
const duplexBridge = {
    listPrinters: async () => printers,
    printerCapabilities: async () => ({ forms: [], duplex: true, color: true }),
};
m.setSelectedPrinter(null);
t = await m.resolveBridgeTarget(duplexBridge);
eq(t.rule.duplex, null, "capable printer still leaves duplex unchosen");

// forms_summary shape: the bridge nests sizes
// under f.size; the summary must show real numbers, never NaN.
const summary = m.formatFormsSummary(
    [
        { name: "A4", size: { width_mm: 210.2, height_mm: 296.8 }, roll: false },
        { name: "Roll 80", size: { width_mm: 80, height_mm: 297 }, roll: true },
        { name: "Letter", width_mm: 216, height_mm: 279, roll: false },
    ],
    "Roll",
);
eq(summary[0], "A4 — 210×297mm", "nested size summary");
eq(summary[1], "Roll 80 — 80×297mm (Roll)", "roll suffix");
eq(summary[2], "Letter — 216×279mm", "flat size tolerated");
eq(m.formatFormsSummary(undefined, "Roll").length, 0, "no forms -> empty");
eq(
    m.formatFormsSummary([{ name: "Custom", size: { width_mm: null, height_mm: "" }, roll: false }], "Roll")[0],
    "Custom — ?×?mm",
    "null/empty dimensions stay unknown instead of 0",
);
eq(
    m.printerPatternFor({ name: "EPSON TM-T88VII", port: " 192.168.1.50 " }, "escpos"),
    "tcp://192.168.1.50:9100",
    "whitespace-padded IP port is trimmed",
);

// Protocol-detection parity: the thermal token
// table and the label rules exist in TWO languages — this page
// fallback and print-core's detect_protocol (the primary, which
// ships on every PrinterInfo). They are one business rule; a
// hand-sync comment is not enforcement, so this suite compares
// the two mechanically and fails on any drift.
const rustSrc = readFileSync(
    new URL("../../../crates/print-core/src/printer.rs", import.meta.url),
    "utf8",
);
const stringsIn = (block) => [...block.matchAll(/"([^"]*)"/g)].map((x) => x[1]);
const htmlSrc = readFileSync(
    new URL("../../../app/dist/index.html", import.meta.url),
    "utf8",
);
const rustTokens = stringsIn(rustSrc.match(/const THERMAL_TOKENS[^=]*= \[([\s\S]*?)\];/)[1]).sort();
const jsTokens = stringsIn(src.match(/const thermalTokens = \[([\s\S]*?)\];/)[1]).sort();
const htmlTokens = stringsIn(htmlSrc.match(/const thermal = \[([\s\S]*?)\];/)[1]).sort();
eq(rustTokens.length, 67, "rust thermal token count");
eq(
    JSON.stringify(jsTokens),
    JSON.stringify(rustTokens),
    "JS and Rust thermal token tables must be identical: " +
        "js-only=" + jsTokens.filter((t) => !rustTokens.includes(t)) +
        " rust-only=" + rustTokens.filter((t) => !jsTokens.includes(t)),
);
eq(JSON.stringify(htmlTokens), JSON.stringify(rustTokens), "HTML and Rust thermal token tables must be identical");
const rustTspl = stringsIn(rustSrc.match(/const TSPL_LABEL_MODELS[^=]*= \[([\s\S]*?)\];/)[1]).sort();
const jsTspl = stringsIn(src.match(/const tsplLabelModels = \[([\s\S]*?)\];/)[1]).sort();
const htmlTspl = stringsIn(htmlSrc.match(/const tsplModels = \[([\s\S]*?)\];/)[1]).sort();
eq(JSON.stringify(jsTspl), JSON.stringify(rustTspl), "JS and Rust TSPL model tables must be identical");
eq(JSON.stringify(htmlTspl), JSON.stringify(rustTspl), "HTML and Rust TSPL model tables must be identical");
const rustOffice = stringsIn(rustSrc.match(/const OFFICE_EXCLUSIONS[^=]*= \[([\s\S]*?)\];/)[1]).sort();
const jsOffice = stringsIn(src.match(/const officeExclusions = \[([\s\S]*?)\];/)[1]).sort();
const htmlOffice = stringsIn(htmlSrc.match(/const officeExclusions = \[([\s\S]*?)\];/)[1]).sort();
eq(JSON.stringify(jsOffice), JSON.stringify(rustOffice), "JS and Rust office-exclusion tables must be identical");
eq(JSON.stringify(htmlOffice), JSON.stringify(rustOffice), "HTML and Rust office-exclusion tables must be identical");

// Run the Control Center fallback functions themselves against a hostile
// virtual-printer payload, not just a source-text assertion.
const functionSlice = (source, startName, nextName) => {
    const start = source.indexOf(`function ${startName}(`);
    const end = source.indexOf(`function ${nextName}(`, start);
    if (start < 0 || end < 0) throw new Error(`cannot extract ${startName}`);
    return source.slice(start, end);
};
const ccSandbox = {};
vm.runInNewContext(
    functionSlice(htmlSrc, "ccProtocol", "ccClass") + "\n" +
    functionSlice(htmlSrc, "ccClass", "ccClassIcon") +
    "\nglobalThis.out = { ccProtocol, ccClass };",
    ccSandbox,
);
eq(ccSandbox.out.ccProtocol({ kind: "virtual", protocol: "escpos" }), "gdi", "Control Center virtual protocol override");
eq(ccSandbox.out.ccClass({ kind: "virtual", printer_class: "thermal" }), "virtual", "Control Center virtual class override");
for (const tok of ['"zdesigner"', '"zebra"', '"zpl"', '"tspl"', '"tsc"', '"epl"', '"cpcl"']) {
    if (!rustSrc.includes(tok)) throw new Error("rust detect_protocol lost label token " + tok);
}
for (const tok of ['"zdesigner"', '"zebra"']) {
    if (!src.includes(tok)) throw new Error("JS detectPrinterType lost label token " + tok);
}
for (const word of ["zpl", "tsc", "tspl", "epl", "cpcl"]) {
    if (!src.includes(`/(^|[^a-z0-9])${word}([^a-z0-9]|$)/`)) {
        throw new Error("JS detectPrinterType missing pre-compiled word regex for " + word);
    }
}
{
    const detectBody = src.slice(src.indexOf("export function detectPrinterType("), src.indexOf("function isIpLiteral("));
    eq(detectBody.includes("new RegExp"), false, "detectPrinterType must not construct RegExp per call");
    // Parity check: pre-compiled word-boundary regexes match the legacy dynamic RegExp oracle.
    const legacyWord = (sig, tok) => new RegExp(`(^|[^a-z0-9])${tok}([^a-z0-9]|$)`).test(sig);
    const parityCases = [
        { name: "ZPL", driver: "" },
        { name: "Printer-ZPL-203", driver: "" },
        { name: "EZPL Printer", driver: "" },
        { name: "ZPL2 Printer", driver: "" },
        { name: "TSC", driver: "" },
        { name: "My/TSC_Label", driver: "" },
        { name: "ATSC Office", driver: "" },
        { name: "TSC200", driver: "" },
        { name: "TSPL Label", driver: "" },
        { name: "MTSPL Office", driver: "" },
        { name: "EPL", driver: "" },
        { name: "Eltron (EPL)", driver: "" },
        { name: "Replit Office", driver: "" },
        { name: "EPL2 Printer", driver: "" },
        { name: "CPCL", driver: "" },
        { name: "Mobile-CPCL-203", driver: "" },
        { name: "MCPCL Office", driver: "" },
        { name: "CPCL2 Printer", driver: "" },
    ];
    for (const p of parityCases) {
        const sig = `${p.name} ${p.driver}`.toLowerCase();
        const expectedMode = legacyWord(sig, "zpl") ? "zpl"
            : (legacyWord(sig, "tsc") || legacyWord(sig, "tspl")) ? "tspl"
            : legacyWord(sig, "epl") ? "epl"
            : legacyWord(sig, "cpcl") ? "cpcl"
            : "gdi";
        eq(m.detectPrinterType(p).mode, expectedMode, `pre-compiled regex parity for ${p.name}`);
    }
}
// The terminal-state contract is single-sourced in the client:
// neither consumer may keep a private copy.
eq(
    JSON.stringify([...m.JOB_TERMINAL_STATES].sort()),
    JSON.stringify(["canceled", "dead_letter", "done", "failed"]),
    "terminal states contract",
);
for (const consumer of ["native_print_handler.js", "pos_native_print.js"]) {
    const consumerSrc = readFileSync(
        new URL("../static/src/js/" + consumer, import.meta.url),
        "utf8",
    );
    if (consumerSrc.includes("const TERMINAL_STATES = new Set")) {
        throw new Error(consumer + " re-declares a private TERMINAL_STATES copy");
    }
    if (!consumerSrc.includes("JOB_TERMINAL_STATES as TERMINAL_STATES")) {
        throw new Error(consumer + " must import the shared JOB_TERMINAL_STATES");
    }
}
eq(m.detectPrinterType({ name: "Xprinter XP-365B", driver: "XP-365B" }).mode, "tspl", "xprinter 365b is tspl label");
eq(m.detectPrinterType({ name: "Xprinter XP-365B", driver: "XP-365B" }).printerClass, "label", "xprinter 365b is label class");
eq(m.detectPrinterType({ name: "Gprinter GP-1324D", driver: "Gprinter" }).mode, "tspl", "gprinter 1324 is tspl label");
eq(m.detectPrinterType({ name: "HP LaserJet Receipt", driver: "HP PCL" }).mode, "gdi", "office laserjet with receipt in name stays gdi");
m.setSelectedPrinter("HP LaserJet");
m.setSelectedPosPrinter("EPSON TM-T88VII");
t = await m.resolveBridgeTarget(bridge, { isPos: true, preferClasses: ["thermal", "label"] });
eq(t.printer.name, "EPSON TM-T88VII", "dedicated pos printer wins for pos even when report printer is selected");
t = await m.resolveBridgeTarget(bridge);
eq(t.printer.name, "HP LaserJet", "report printer stays HP LaserJet");
m.setSelectedPosPrinter(null);
m.setSelectedPrinter(null);
// End-to-end handler regression tests: quiet-retry status sync,
// qweb-text 1 MiB/empty guard, and ASCII-safe <=128-char idempotency keys.
const rpcCalls = [];
const notices = [];
let registeredHandler = null;
const handlerSrc = readFileSync(
    new URL("../static/src/js/native_print_handler.js", import.meta.url),
    "utf8",
)
    .replace('import { registry } from "@web/core/registry";',
        'const registry = { category: () => ({ add: (_k, fn) => { globalThis.__npHandler = fn; } }) };')
    .replace('import { rpc } from "@web/core/network/rpc";',
        'const rpc = async (url, params) => { globalThis.__npRpcCalls.push({ url, params }); return { ok: true }; };')
    .replace('import { user } from "@web/core/user";',
        'const user = globalThis.__npUser;')
    .replace('import { _t } from "@web/core/l10n/translation";',
        'const _t = (s, ...args) => args.reduce((acc, v) => acc.replace("%s", String(v)), s);')
    .replace('from "./native_print_client";',
        `from "${tmpUrl}";`);
globalThis.__npRpcCalls = rpcCalls;
globalThis.__npUser = { context: {} };
const tmpHandler = join(tmpDir, "np_handler_test.mjs");
writeFileSync(tmpHandler, handlerSrc);
const h = await import(pathToFileURL(tmpHandler).href);
registeredHandler = globalThis.__npHandler;
const testEnv = {
    services: {
        notification: { add: (msg, opts) => notices.push({ msg, type: opts && opts.type }) },
    },
};

const origSetTimeout = globalThis.setTimeout;
globalThis.setTimeout = (fn) => { fn(); return 0; };
try {
    // 1. Quiet retry failed -> done must post `done` (not leave server `failed`).
    rpcCalls.length = 0;
    notices.length = 0;
    let pollIdx = 0;
    const retryDoneBridge = {
        jobDetail: async () => {
            pollIdx++;
            if (pollIdx === 1) return { state: "failed", errorCode: "BUSY", pages: 1, attempts: 1 };
            return { state: "done", errorCode: null, pages: 2, attempts: 2 };
        },
    };
    const resDone = await h.watchJobToCompletion(
        retryDoneBridge,
        "job-retry-done",
        { reportName: "account.report_invoice", printerName: "HP", printerPattern: "HP", byteSize: 100 },
        testEnv,
    );
    eq(resDone, "done", "quiet-retry recovery returns done");
    eq(rpcCalls.length, 1, "quiet-retry recovery posts one final status");
    eq(rpcCalls[0].params.state, "done", "quiet-retry recovery syncs done to server");
    eq(rpcCalls[0].params.pages, 2, "quiet-retry recovery syncs updated pages");
    eq(notices.length, 1, "quiet-retry recovery emits one toast");
    eq(notices[0].type, "success", "quiet-retry recovery toast is success");

    // 2. Quiet retry failed -> canceled must post `canceled` and emit no danger toast.
    rpcCalls.length = 0;
    notices.length = 0;
    let cancelIdx = 0;
    const retryCancelBridge = {
        jobDetail: async () => {
            cancelIdx++;
            if (cancelIdx === 1) return { state: "failed", errorCode: "BUSY", pages: 1, attempts: 1 };
            return { state: "canceled", errorCode: null, pages: 1, attempts: 1 };
        },
    };
    const resCancel = await h.watchJobToCompletion(
        retryCancelBridge,
        "job-retry-cancel",
        { reportName: "account.report_invoice", printerName: "HP", printerPattern: "HP", byteSize: 100 },
        testEnv,
    );
    eq(resCancel, "canceled", "quiet-retry canceled returns canceled");
    eq(rpcCalls.length, 1, "quiet-retry canceled posts one status");
    eq(rpcCalls[0].params.state, "canceled", "quiet-retry canceled syncs canceled");
    eq(notices.length, 0, "quiet-retry canceled emits no danger toast");

    // 2b. buildReportUrl uses @web/core/user context on Odoo 18/19/20 and wizard options.
    globalThis.__npUser.context = { lang: "ar_001", allowed_company_ids: [2] };
    const urlWithUserCtx = h.buildReportUrl({ report_name: "account.report_invoice", context: { active_ids: [5] } }, "pdf", {});
    eq(urlWithUserCtx.includes("/report/pdf/account.report_invoice/5?context="), true, "buildReportUrl includes docids and context");
    eq(decodeURIComponent(urlWithUserCtx).includes('"lang":"ar_001"'), true, "buildReportUrl inherits Odoo user.context");
    const wizardUrl = h.buildReportUrl({ report_name: "sale.report", data: { date_from: "2026-01-01" } }, "pdf", {});
    eq(wizardUrl.includes("?options="), true, "wizard data encodes options query param");
    eq(h.reportSource(null).ids, "", "reportSource tolerates null action");
    globalThis.__npUser.context = {};

    // 2c. Already-notified failed job does not repeat 900ms recheck IPC on subsequent polls.
    rpcCalls.length = 0;
    notices.length = 0;
    let failedPolls = 0;
    const origDateNowRetry = Date.now;
    let fakeRetryNow = origDateNowRetry();
    Date.now = () => fakeRetryNow;
    try {
        const steadyFailBridge = {
            jobDetail: async () => {
                failedPolls++;
                // Poll 1 + its single 900ms recheck -> 2 calls; Poll 2 must NOT recheck (3rd call total),
                // then advance past 5-minute progress cap to exit cleanly.
                if (failedPolls >= 3) fakeRetryNow += 6 * 60 * 1000;
                return { state: "failed", errorCode: "OFFLINE", pages: 1, attempts: 1 };
            },
        };
        await h.watchJobToCompletion(
            steadyFailBridge,
            "job-steady-fail",
            { reportName: "account.report_invoice", printerName: "HP", printerPattern: "HP", byteSize: 100 },
            testEnv,
        );
        eq(failedPolls, 3, "already-notified failure skips duplicate recheck IPC");
        eq(notices.length, 1, "steady failure notifies user only once");
    } finally {
        Date.now = origDateNowRetry;
    }
} finally {
    globalThis.setTimeout = origSetTimeout;
}

// 3. qweb-text on escpos: empty and >1 MiB fall back without calling bridge.printText.
const origFetch = globalThis.fetch;
try {
    let textPrintCalls = 0;
    globalThis.window.__NATIVE_PRINT__ = {
        listPrinters: async () => [{ name: "EPSON TM-T88VII", driver: "EPSON", kind: "local", is_default: true }],
        printerCapabilities: async () => ({ forms: [], color: false }),
        printText: async () => { textPrintCalls++; return "ok"; },
    };
    rpcCalls.length = 0;
    globalThis.fetch = async () => ({ ok: true, text: async () => "" });
    const emptyRes = await registeredHandler({ report_type: "qweb-text", report_name: "r.empty" }, {}, testEnv);
    eq(emptyRes, undefined, "empty qweb-text falls back");
    eq(textPrintCalls, 0, "empty qweb-text never calls printText");
    eq(rpcCalls.length, 0, "empty qweb-text never posts queued status");

    globalThis.fetch = async () => ({ ok: true, text: async () => "x".repeat(1024 * 1024 + 1) });
    const bigRes = await registeredHandler({ report_type: "qweb-text", report_name: "r.big" }, {}, testEnv);
    eq(bigRes, undefined, "oversized >1MiB qweb-text falls back");
    eq(textPrintCalls, 0, "oversized qweb-text never calls printText");

    globalThis.fetch = async () => { throw new Error("network reset"); };
    const netErrRes = await registeredHandler({ report_type: "qweb-text", report_name: "r.neterr" }, {}, testEnv);
    eq(netErrRes, undefined, "rejected qweb-text fetch falls back cleanly to undefined");

    // 4. Dynamic report with long non-ASCII report_name produces a valid <=128-char ASCII key.
    let capturedKey = null;
    globalThis.window.__NATIVE_PRINT__ = {
        listPrinters: async () => [{ name: "HP LaserJet", driver: "HP PCL", kind: "local", is_default: true }],
        printerCapabilities: async () => ({ forms: [], color: true }),
        printPdf: async (req) => { capturedKey = req.idempotencyKey; return req.idempotencyKey; },
        jobDetail: async () => ({ state: "done", pages: 1, attempts: 1 }),
    };
    globalThis.fetch = async () => ({
        ok: true,
        arrayBuffer: async () => new Uint8Array([0x25, 0x50, 0x44, 0x46, 0x2d, 0x31, 0x2e, 0x34]).buffer,
    });
    const longName = "فاتورة " + "a".repeat(150);
    const pdfRes = await registeredHandler({ report_type: "qweb-pdf", report_name: longName }, {}, testEnv);
    eq(pdfRes, true, "dynamic long non-ASCII report prints");
    eq(capturedKey.length <= 128, true, "idempotencyKey length <= 128");
    eq(/^[!-~]+$/.test(capturedKey), true, "idempotencyKey is printable ASCII");

    // 4b. Bridge failure must not leak raw exception details to UI toast and must tolerate null throw.
    notices.length = 0;
    rpcCalls.length = 0;
    globalThis.window.__NATIVE_PRINT__ = {
        listPrinters: async () => [{ name: "HP LaserJet", driver: "HP PCL", kind: "local", is_default: true }],
        printerCapabilities: async () => ({ forms: [], color: true }),
        printPdf: async () => { throw new Error("INTERNAL_RUST_SECRET_TRACE"); },
    };
    const errRes = await registeredHandler(
        { report_type: "qweb-pdf", report_name: "r.err", context: { active_model: "account.move", active_ids: [7] } },
        {},
        testEnv,
    );
    eq(errRes, undefined, "bridge printPdf error falls back to download");
    eq(notices.length, 1, "bridge error emits one warning toast");
    eq(notices[0].msg.includes("INTERNAL_RUST_SECRET_TRACE"), false, "raw exception text is not leaked to UI");
    eq(rpcCalls.length, 1, "bridge error posts failed job status");
    eq(rpcCalls[0].params.error_code, "BRIDGE_ERROR", "bridge error posts BRIDGE_ERROR code");
    eq(rpcCalls[0].params.report_name, "r.err", "bridge error preserves report_name for job log");
    eq(rpcCalls[0].params.source_ids, "7", "bridge error preserves source_ids for reprint");

    notices.length = 0;
    globalThis.window.__NATIVE_PRINT__.printPdf = async () => { throw null; };
    const nullErrRes = await registeredHandler({ report_type: "qweb-pdf", report_name: "r.nullerr" }, {}, testEnv);
    eq(nullErrRes, undefined, "null bridge rejection falls back cleanly without TypeError");

    // 4c. A server without wkhtmltopdf answers /report/pdf with HTML.
    // The native handler must print the text version instead of letting
    // Odoo show the HTML fallback and its "no PDF engine" warning.
    notices.length = 0;
    rpcCalls.length = 0;
    let textFallbackPrint = null;
    const prevDoc = globalThis.document;
    globalThis.document = {
        createElement: () => ({
            width: 0,
            height: 0,
            getContext: () => ({
                measureText: () => ({ width: 7.8 }),
                fillRect() {},
                fillText() {},
            }),
            toBlob(cb) {
                cb({ arrayBuffer: async () => new Uint8Array([137, 80, 78, 71]).buffer });
            },
        }),
    };
    try {
        globalThis.window.__NATIVE_PRINT__ = {
            listPrinters: async () => [
                { name: "OneNote (Desktop)", driver: "Microsoft", kind: "local", is_default: true },
            ],
            printerCapabilities: async () => ({ forms: [], color: true }),
            printImage: async (req) => { textFallbackPrint = req; return req.idempotencyKey; },
            jobDetail: async () => ({ state: "done", pages: 1, attempts: 1 }),
        };
        let fetchCount = 0;
        globalThis.fetch = async (url) => {
            fetchCount++;
            if (String(url).includes("/report/pdf/")) {
                return {
                    ok: true,
                    arrayBuffer: async () =>
                        new TextEncoder().encode("<html>no PDF engine on this system</html>").buffer,
                };
            }
            // Odoo's text converter actually renders the QWeb template.
            return {
                ok: true,
                text: async () =>
                    "<html><body><div>INVOICE INV/2026/0006</div><div>TOTAL 100.00 SR</div></body></html>",
            };
        };
        const noPdfRes = await registeredHandler(
            {
                report_type: "qweb-pdf",
                report_name: "account.report_invoice",
                context: { active_ids: [6] },
            },
            {},
            testEnv,
        );
        eq(noPdfRes, true, "missing PDF engine still prints natively");
        eq(fetchCount, 2, "PDF and text converter are both fetched");
        eq(textFallbackPrint !== null, true, "text version is sent to the printer");
        eq(textFallbackPrint.mode, "gdi", "text version is rendered as a GDI image");
        eq(textFallbackPrint.bytes.length > 0, true, "PNG bytes are produced locally");
        eq(
            rpcCalls.some((c) => c.params.state === "queued"),
            true,
            "the fallback job is logged to Odoo",
        );
        eq(
            notices.some((n) => /no PDF engine/i.test(n.msg)),
            true,
            "the user is told why the text version was printed",
        );

        // Odoo may already have converted the action to qweb-html: that
        // must still print natively, without the HTML report ever opening.
        notices.length = 0;
        textFallbackPrint = null;
        const htmlActionRes = await registeredHandler(
            {
                type: "ir.actions.report",
                report_type: "qweb-html",
                report_name: "account.report_invoice",
                context: { active_ids: [7] },
            },
            {},
            testEnv,
        );
        eq(htmlActionRes, true, "qweb-html report is handled natively");
        eq(textFallbackPrint !== null, true, "qweb-html report reaches the printer");
        eq(
            h.reportHtmlToPlainText("<div>A</div><div>B<br>C</div>"),
            "A\nB\nC",
            "QWeb HTML becomes printable lines",
        );
    } finally {
        globalThis.document = prevDoc;
    }
} finally {
    globalThis.fetch = origFetch;
    delete globalThis.window.__NATIVE_PRINT__;
}

// 5. Regression tests: hung fetch AbortController timeout & stale inFlightReports recovery.
{
    const origFetch2 = globalThis.fetch;
    const origSetTimeout2 = globalThis.setTimeout;
    const origClearTimeout2 = globalThis.clearTimeout;
    const origDateNow = Date.now;
    let fakeNow = origDateNow();
    Date.now = () => fakeNow;
    try {
        globalThis.window.__NATIVE_PRINT__ = {
            listPrinters: async () => [{ name: "HP LaserJet", driver: "HP PCL", kind: "local", is_default: true }],
            printerCapabilities: async () => ({ forms: [], color: true }),
            printPdf: async (req) => req.idempotencyKey,
            jobDetail: async () => ({ state: "done", pages: 1, attempts: 1 }),
        };

        // 5a. Hung fetch is aborted by AbortController timeout, falls back to undefined, and releases lock.
        let sawAborted = false;
        let clearedTimer = false;
        globalThis.setTimeout = (fn, ms) => {
            if (ms === h.REPORT_FETCH_TIMEOUT_MS) {
                queueMicrotask(fn);
                return 4242;
            }
            return origSetTimeout2(fn, ms);
        };
        globalThis.clearTimeout = (id) => {
            if (id === 4242) clearedTimer = true;
            return origClearTimeout2(id);
        };
        globalThis.fetch = (_url, opts) => new Promise((_resolve, reject) => {
            opts.signal.addEventListener("abort", () => {
                sawAborted = true;
                reject(new Error("AbortError"));
            });
        });
        const hungRes = await registeredHandler(
            { report_type: "qweb-pdf", report_name: "r.hung", context: { active_ids: [101] } },
            {},
            testEnv,
        );
        eq(hungRes, undefined, "hung fetch aborts and falls back to Odoo download");
        eq(sawAborted, true, "hung fetch received AbortSignal abort");
        eq(clearedTimer, true, "fetch timeout timer is cleared in finally");

        // Next call for same report/record succeeds (lock was released).
        globalThis.setTimeout = origSetTimeout2;
        globalThis.clearTimeout = origClearTimeout2;
        globalThis.fetch = async () => ({
            ok: true,
            arrayBuffer: async () => new Uint8Array([0x25, 0x50, 0x44, 0x46, 0x2d, 0x31, 0x2e, 0x34]).buffer,
        });
        const retryAfterTimeout = await registeredHandler(
            { report_type: "qweb-pdf", report_name: "r.hung", context: { active_ids: [101] } },
            {},
            testEnv,
        );
        eq(retryAfterTimeout, true, "report prints normally after prior timeout released lock");

        // 5b. Stale inFlightReports entry expires and late first finally does NOT clear newer lock.
        let fetchCount = 0;
        const resolvers = [];
        globalThis.fetch = () => {
            fetchCount++;
            return new Promise((resolve) => {
                resolvers.push(() => resolve({
                    ok: true,
                    arrayBuffer: async () => new Uint8Array([0x25, 0x50, 0x44, 0x46, 0x2d, 0x31, 0x2e, 0x34]).buffer,
                }));
            });
        };
        const staleAction = { report_type: "qweb-pdf", report_name: "r.stale", context: { active_ids: [202] } };
        const pFirst = registeredHandler(staleAction, {}, testEnv);
        await new Promise((r) => origSetTimeout2(r, 10));
        eq(fetchCount, 1, "first submission started fetch");

        // Duplicate while fresh is suppressed.
        const pDup = await registeredHandler(staleAction, {}, testEnv);
        eq(pDup, true, "fresh duplicate returns true");
        eq(fetchCount, 1, "fresh duplicate does not start a second fetch");

        // Advance clock past IN_FLIGHT_STALE_MS -> new submission is allowed.
        fakeNow += h.IN_FLIGHT_STALE_MS + 1000;
        const pSecond = registeredHandler(staleAction, {}, testEnv);
        await new Promise((r) => origSetTimeout2(r, 10));
        eq(fetchCount, 2, "stale in-flight entry allows new submission");

        // Settle first (stale) submission while second is still running.
        resolvers[0]();
        await pFirst;

        // A third click while second is still fresh must STILL be deduped (first finally did not clobber lock).
        const pDup2 = await registeredHandler(staleAction, {}, testEnv);
        eq(pDup2, true, "second submission lock survives stale first finally");
        eq(fetchCount, 2, "stale finally did not release active second lock");

        // Settle second submission.
        resolvers[1]();
        eq(await pSecond, true, "second submission completes");

        // 5c. When inFlightReports exceeds 20 entries, stale entries are pruned.
        const sweepResolvers = [];
        globalThis.fetch = () => new Promise((resolve) => {
            sweepResolvers.push(() => resolve({
                ok: true,
                arrayBuffer: async () => new Uint8Array([0x25, 0x50, 0x44, 0x46, 0x2d, 0x31, 0x2e, 0x34]).buffer,
            }));
        });
        const stalePending = [];
        for (let idx = 0; idx < 21; idx++) {
            stalePending.push(registeredHandler(
                { report_type: "qweb-pdf", report_name: `r.sweep.${idx}`, context: { active_ids: [idx + 1] } },
                {},
                testEnv,
            ));
        }
        await new Promise((r) => origSetTimeout2(r, 10));
        eq(sweepResolvers.length, 21, "21 in-flight entries created");
        // Advance clock past IN_FLIGHT_STALE_MS and submit a 22nd report to trigger sweep.
        fakeNow += h.IN_FLIGHT_STALE_MS + 1000;
        const freshPromise = registeredHandler(
            { report_type: "qweb-pdf", report_name: "r.sweep.fresh", context: { active_ids: [999] } },
            {},
            testEnv,
        );
        await new Promise((r) => origSetTimeout2(r, 10));
        eq(sweepResolvers.length, 22, "22nd fresh submission starts and sweeps stale entries");
        // A duplicate of the fresh 22nd report is still deduped (sweep kept fresh entry).
        const freshDup = await registeredHandler(
            { report_type: "qweb-pdf", report_name: "r.sweep.fresh", context: { active_ids: [999] } },
            {},
            testEnv,
        );
        eq(freshDup, true, "fresh entry survives stale sweep");
        eq(sweepResolvers.length, 22, "fresh duplicate did not start a 23rd fetch");
        for (const r of sweepResolvers) r();
        await Promise.all([...stalePending, freshPromise]);
    } finally {
        Date.now = origDateNow;
        globalThis.fetch = origFetch2;
        globalThis.setTimeout = origSetTimeout2;
        globalThis.clearTimeout = origClearTimeout2;
        delete globalThis.window.__NATIVE_PRINT__;
    }
}

// 6. Systray regression tests: onWillUnmount cleanup & maybeSyncDeviceCaps throttle.
// Strictly stubs OWL 3 APIs only (proxy / usePlugin) so any OWL 2 regression fails loudly.
{
    let unmountHook = null;
    let mountedHook = null;
    const listeners = [];
    const intervals = [];
    const clearedIntervals = [];
    const owlStub = [
        "class Component {}",
        "const onMounted = (fn) => { globalThis.__npOnMounted = fn; };",
        "const onWillUnmount = (fn) => { globalThis.__npOnWillUnmount = fn; };",
        "const proxy = (s) => new Proxy(s, { set(t, k, v) { globalThis.__npProxyWrites = (globalThis.__npProxyWrites || 0) + 1; t[k] = v; return true; } });",
        "const usePlugin = () => ({ add: () => {}, doAction: () => {} });",
    ].join(" ");
    const systraySrc = readFileSync(
        new URL("../static/src/js/native_print_systray.js", import.meta.url),
        "utf8",
    )
        .replace(/import\s*\{[^}]*\}\s*from\s*["']@odoo\/owl["'];?/, owlStub)
        .replace(/import\s*\{[^}]*\}\s*from\s*["']@web\/core\/registry["'];?/, 'const registry = { category: () => ({ add: () => {} }) };')
        .replace(/import\s*\{[^}]*\}\s*from\s*["']@web\/core\/network\/rpc["'];?/, 'const rpc = async (url, params) => { globalThis.__npSystrayRpcs.push({ url, params }); if (url === "/native_print/overview") return globalThis.__npOverviewResp || { ok: true, active_jobs: 0, done_jobs: 0, failed_jobs: 0 }; return { ok: true }; };')
        .replace(/import\s*\{[^}]*\}\s*from\s*["']@web\/core\/l10n\/translation["'];?/, 'const _t = (s) => s;')
        .replace(/import\s*\{[^}]*\}\s*from\s*["']@web\/webclient\/actions\/action_plugin["'];?/, 'const ActionPlugin = class {};')
        .replace(/import\s*\{[^}]*\}\s*from\s*["']@web\/core\/notifications\/notification_plugin["'];?/, 'const NotificationPlugin = class {};')
        .replace(/import\s*\{[^}]*\}\s*from\s*["']@web\/[^"']+["'];?/g, '')
        .replace('from "./native_print_client";', `from "${tmpUrl}";`);
    globalThis.__npSystrayRpcs = [];
    const tmpSystray = join(tmpDir, "np_systray_test.mjs");
    writeFileSync(tmpSystray, systraySrc);
    const sMod = await import(pathToFileURL(tmpSystray).href);

    const prevLocalStorage = globalThis.localStorage;
    const prevDocument = globalThis.document;
    globalThis.localStorage = globalThis.window.localStorage;
    globalThis.document = { visibilityState: "visible" };
    globalThis.window.addEventListener = (ev, fn) => listeners.push({ ev, fn });
    globalThis.window.removeEventListener = (ev, fn) => {
        const idx = listeners.findIndex((x) => x.ev === ev && x.fn === fn);
        if (idx >= 0) listeners.splice(idx, 1);
    };
    globalThis.window.setInterval = (fn, ms) => { intervals.push({ fn, ms }); return 777; };
    globalThis.window.clearInterval = (id) => { clearedIntervals.push(id); };

    try {
        let capCalls = 0;
        globalThis.window.__NATIVE_PRINT__ = {
            printerCapabilities: async () => {
                capCalls++;
                return { forms: [{ name: "A4", size: { width_mm: 210, height_mm: 297 }, roll: false }], bins: ["Tray 1"] };
            },
        };
        const comp = new sMod.NativePrintSystray();
        comp.setup();
        mountedHook = globalThis.__npOnMounted;
        unmountHook = globalThis.__npOnWillUnmount;

        // Verify mounted registers click + job_updated + interval and onWillUnmount cleans all up.
        globalThis.window.dispatchEvent = (ev) => {
            for (const l of listeners.slice()) {
                if (l.ev === ev.type) l.fn(ev);
            }
            return true;
        };
        mountedHook();
        eq(listeners.length, 2, "onMounted registers click and job_updated listeners");
        eq(listeners.some((l) => l.ev === "native_print:job_updated"), true, "onMounted listens for native_print:job_updated");
        eq(intervals.length, 1, "onMounted registers refresh interval");

        // Event-driven systray counter refresh: watchJobToCompletion -> native_print:job_updated -> refreshStats().
        globalThis.__npOverviewResp = { ok: true, active_jobs: 0, done_jobs: 7, failed_jobs: 2 };
        globalThis.__npSystrayRpcs.length = 0;
        const prevSetTimeoutEv = globalThis.setTimeout;
        globalThis.setTimeout = (fn) => { fn(); return 0; };
        try {
            await h.watchJobToCompletion(
                { jobDetail: async () => ({ state: "done", pages: 1, attempts: 1 }) },
                "job-ev-done",
                { reportName: "account.report_invoice", printerName: "HP", printerPattern: "HP", byteSize: 50 },
                testEnv,
            );
        } finally {
            globalThis.setTimeout = prevSetTimeoutEv;
        }
        await new Promise((r) => setTimeout(r, 10));
        eq(comp.state.doneJobs, 7, "native_print:job_updated immediately refreshes doneJobs");
        eq(comp.state.failedJobs, 2, "native_print:job_updated immediately refreshes failedJobs");

        // OWL 3 re-render hygiene: repeated 60s background polls with unchanged
        // printers and job counters must cause zero reactive proxy writes.
        const steadyOverview = {
            defaultPrinter: "HP LaserJet",
            printers: [{ name: "HP LaserJet", kind: "local", port: "USB001", protocol: "gdi", printer_class: "office" }],
        };
        globalThis.window.__NATIVE_PRINT__.appOverview = async () => ({
            defaultPrinter: steadyOverview.defaultPrinter,
            printers: steadyOverview.printers.map((p) => ({ ...p })),
        });
        await comp.refreshStats();
        const prevDiscoveredRef = comp.state.discoveredPrinters;
        globalThis.__npProxyWrites = 0;
        await comp.refreshStats();
        eq(globalThis.__npProxyWrites, 0, "unchanged background refreshStats triggers zero reactive state mutations");
        eq(comp.state.discoveredPrinters, prevDiscoveredRef, "unchanged poll preserves discoveredPrinters array reference");
        // When a counter or printer changes, reactive state updates normally.
        globalThis.__npOverviewResp = { ok: true, active_jobs: 1, done_jobs: 7, failed_jobs: 2 };
        await comp.refreshStats();
        eq(globalThis.__npProxyWrites, 1, "changed active_jobs triggers exactly one reactive write");
        eq(comp.state.activeJobs, 1, "activeJobs updated reactively");
        delete globalThis.window.__NATIVE_PRINT__.appOverview;

        const printerList = [{ name: "HP LaserJet", protocol: "gdi", printer_class: "office" }];
        await comp.maybeSyncDeviceCaps(printerList, false);
        eq(capCalls, 1, "first maybeSyncDeviceCaps queries driver capabilities");
        await comp.maybeSyncDeviceCaps(printerList, false);
        eq(capCalls, 1, "background poll within 6h skips redundant printerCapabilities call");
        await comp.maybeSyncDeviceCaps(printerList, true);
        eq(capCalls, 2, "manual refresh (force=true) queries printerCapabilities");
        await comp.maybeSyncDeviceCaps([{ name: "HP LaserJet", protocol: "escpos", printer_class: "thermal" }], false);
        eq(capCalls, 3, "protocol/class change triggers fresh printerCapabilities query");

        unmountHook();
        eq(listeners.length, 0, "onWillUnmount removes click and job_updated listeners");
        // After unmount, dispatching native_print:job_updated must not call /native_print/overview.
        const overviewCountAfterUnmount = globalThis.__npSystrayRpcs.filter((c) => c.url === "/native_print/overview").length;
        m.notifyJobUpdated();
        await new Promise((r) => setTimeout(r, 10));
        eq(
            globalThis.__npSystrayRpcs.filter((c) => c.url === "/native_print/overview").length,
            overviewCountAfterUnmount,
            "unmounted systray ignores native_print:job_updated",
        );
        eq(clearedIntervals.includes(777), true, "onWillUnmount clears refresh interval");
        // Corrupted non-object localStorage value does not crash maybeSyncDeviceCaps.
        globalThis.localStorage.setItem("npCapsSyncV1", "null");
        await comp.maybeSyncDeviceCaps(printerList, false);
        eq(capCalls, 4, "corrupted null npCapsSyncV1 storage recovers safely");

        // Escape key closes open systray dropdown.
        comp.state.isOpen = true;
        let stopped = false;
        comp.onKeyDown({ key: "Escape", stopPropagation: () => { stopped = true; } });
        eq(comp.state.isOpen, false, "Escape key closes systray menu");
        eq(stopped, true, "Escape key stops propagation when closing");

        eq(comp._unmounted, true, "onWillUnmount marks component unmounted");
    } finally {
        globalThis.localStorage = prevLocalStorage;
        globalThis.document = prevDocument;
        delete globalThis.window.__NATIVE_PRINT__;
    }
}

// 7. Security & IP-literal edge cases: safeErrorCode() and isIpLiteral(":::").
{
    eq(m.safeErrorCode("OFFLINE"), "OFFLINE", "safeErrorCode preserves clean code");
    eq(m.safeErrorCode("  ERR_01.timeout-2  "), "ERR_01.timeout-2", "safeErrorCode trims valid token");
    eq(m.safeErrorCode("ERR\nSTACK_TRACE", "failed"), "failed", "safeErrorCode rejects multi-line string");
    eq(m.safeErrorCode("Error: boom at native_print.rs:42", "failed"), "failed", "safeErrorCode rejects trace string");
    eq(m.safeErrorCode("A".repeat(65)), "error", "safeErrorCode rejects >64-char string");
    eq(m.safeErrorCode(null), "error", "safeErrorCode handles null");

    const ipSandbox = {};
    vm.runInNewContext(
        src.match(/function isIpLiteral\([\s\S]*?\n\}/)[0] + "\nglobalThis.isIpLiteral = isIpLiteral;",
        ipSandbox,
    );
    eq(ipSandbox.isIpLiteral(":::"), false, 'isIpLiteral(":::") must return false');
    eq(ipSandbox.isIpLiteral("1::2::3"), false, "isIpLiteral rejects multiple :: compressions");
    eq(ipSandbox.isIpLiteral(":1"), false, "isIpLiteral rejects leading single colon");
    eq(ipSandbox.isIpLiteral("1:"), false, "isIpLiteral rejects trailing single colon");
    eq(ipSandbox.isIpLiteral("::1"), true, "isIpLiteral accepts ::1");
    eq(m.printerPatternFor({ name: "Label", port: ":::" }, "zpl"), "Label", 'printerPatternFor rejects ":::" as IP literal');
}

// 8. Android v1.5.7 bridge parity regression tests: wire states, UUID polling,
// explicit printer routing, PrinterInfo/PrinterCapabilities, and systray metadata.
{
    // 8a. normalizeBridgeJobState & resolveBridgeJobId
    eq(m.normalizeBridgeJobState("completed"), "done", "Android completed -> done");
    eq(m.normalizeBridgeJobState("cancelled"), "canceled", "Android cancelled -> canceled");
    eq(m.normalizeBridgeJobState("submitted"), "printing", "Android submitted -> printing");
    eq(m.normalizeBridgeJobState("in_progress"), "printing", "Android in_progress -> printing");
    eq(m.normalizeBridgeJobState("not_found"), null, "Android not_found -> null");
    eq(m.normalizeBridgeJobState("  COMPLETED "), "done", "wire state trims and lowercases");
    eq(m.normalizeBridgeJobState(null), null, "non-string state -> null");
    eq(m.resolveBridgeJobId({ id: "uuid-123" }, "idem-1"), "uuid-123", "Android object id extracted");
    eq(m.resolveBridgeJobId({ job_id: "uuid-456" }, "idem-1"), "uuid-456", "Android job_id extracted");
    eq(m.resolveBridgeJobId({ jobId: "uuid-789" }, "idem-1"), "uuid-789", "Android jobId extracted");
    eq(m.resolveBridgeJobId("idem-1", "fallback"), "idem-1", "Desktop string id preserved");
    eq(m.resolveBridgeJobId(null, "fallback"), "fallback", "Null enqueue result falls back to key");

    // 8b. Android PrinterInfo / PrinterCapabilities shapes
    eq(m.detectPrinterType({ name: "BT Thermal", protocol: "escpos" }).printerClass, "thermal", "escpos without printer_class infers thermal");
    eq(m.detectPrinterType({ name: "Zebra BT", protocol: "zpl" }).printerClass, "label", "zpl without printer_class infers label");
    eq(m.detectPrinterType({ name: "Mobile BT", protocol: "cpcl" }).printerClass, "label", "cpcl without printer_class infers label");
    eq(m.detectPrinterType({ name: "Office IPP", protocol: "pdf" }).mode, "gdi", "Android pdf protocol maps to gdi");
    eq(m.detectPrinterType({ name: "EPSON TM-T88VII", protocol: "auto" }).mode, "escpos", "Android auto protocol falls through to heuristic");
    eq(m.formatFormsSummary(["80mm Roll", "A4"], "Roll")[0], "80mm Roll", "Android string forms preserved");
    eq(m.rollWidthMm({ forms: ["80mm Roll", "A4"] }), 80, "Android string roll width parsed");
    eq(m.rollWidthMm({ forms: [], paper_width_mm: 58 }), 58, "Android paper_width_mm fallback");
    eq(m.rollWidthMm({ forms: [], paper_width_mm: 210 }), null, "A4 paper_width_mm is not a roll");

    // 8c. resolveBridgeTarget forwards { force: true } and respects supports_color: false
    m.setSelectedPrinter("");
    m.setSelectedPosPrinter("");
    let receivedForce = null;
    const androidBridge = {
        listPrinters: async (opts) => {
            receivedForce = Boolean(opts && opts.force);
            return [{ name: "Star BT", transport: "bluetooth_spp", protocol: "escpos", is_default: true }];
        },
        printerCapabilities: async () => ({ forms: ["80mm Roll"], paper_width_mm: 80, supports_color: false }),
    };
    const androidTarget = await m.resolveBridgeTarget(androidBridge, { isPos: true, preferClasses: ["thermal"], force: true });
    eq(receivedForce, true, "resolveBridgeTarget forwards force: true to listPrinters");
    eq(androidTarget.rule.color, false, "supports_color: false sets rule.color to false");
    eq(androidTarget.rule.media.roll_width_mm, 80, "Android roll_width_mm populated");

    // 8d. PDF & text report handlers pass `printer`, poll by returned UUID, and normalize wire states.
    const origFetch8 = globalThis.fetch;
    const origSetTimeout8 = globalThis.setTimeout;
    globalThis.setTimeout = (fn) => { fn(); return 0; };
    try {
        rpcCalls.length = 0;
        let sentPdfReq = null;
        const polledPdfIds = [];
        let pdfPollIdx = 0;
        globalThis.window.__NATIVE_PRINT__ = {
            listPrinters: async () => [{ name: "Android IPP", transport: "ipp", address: "192.168.1.20", protocol: "pdf", is_default: true }],
            printerCapabilities: async () => ({ forms: ["A4"], supports_color: true }),
            printPdf: async (req) => { sentPdfReq = req; return { id: "android-uuid-99", state: "queued" }; },
            jobDetail: async (id) => {
                polledPdfIds.push(id);
                const states = ["submitted", "in_progress", "completed"];
                return { id, state: states[Math.min(pdfPollIdx++, states.length - 1)], pages: 1, attempts: 1 };
            },
        };
        globalThis.fetch = async () => ({
            ok: true,
            arrayBuffer: async () => new Uint8Array([0x25, 0x50, 0x44, 0x46, 0x2d, 0x31, 0x2e, 0x34]).buffer,
        });
        const okPdf = await registeredHandler({ report_type: "qweb-pdf", report_name: "account.report_invoice" }, {}, testEnv);
        await new Promise((r) => origSetTimeout8(r, 15));
        eq(okPdf, true, "Android PDF print succeeds");
        eq(sentPdfReq.printer, "Android IPP", "printPdf passes explicit printer name for Android");
        eq(sentPdfReq.printerPattern, "Android IPP", "printPdf preserves printerPattern");
        eq(polledPdfIds[0], "android-uuid-99", "watchJobToCompletion polls Android UUID");
        const printingRpc = rpcCalls.find((c) => c.url === "/native_print/job_status" && c.params.state === "printing");
        const doneRpc = rpcCalls.find((c) => c.url === "/native_print/job_status" && c.params.state === "done");
        eq(Boolean(printingRpc), true, "Android submitted/in_progress normalized to printing");
        eq(Boolean(doneRpc), true, "Android completed normalized to done");
        eq(doneRpc.params.idempotency_key, sentPdfReq.idempotencyKey, "done RPC preserves original idempotency_key");

        // qweb-text on Android passes `printer`, polls by UUID, normalizes `cancelled` -> `canceled`.
        rpcCalls.length = 0;
        let sentTextReq = null;
        const polledTextIds = [];
        globalThis.window.__NATIVE_PRINT__ = {
            listPrinters: async () => [{ name: "BT Thermal", transport: "bluetooth_spp", protocol: "escpos", is_default: true }],
            printerCapabilities: async () => ({ forms: ["80mm Roll"], paper_width_mm: 80, supports_color: false }),
            printText: async (req) => { sentTextReq = req; return { id: "android-txt-uuid-7", state: "queued" }; },
            jobDetail: async (id) => { polledTextIds.push(id); return { id, state: "cancelled", pages: 1, attempts: 1 }; },
        };
        globalThis.fetch = async () => ({ ok: true, text: async () => "Hello Android" });
        const okTxt = await registeredHandler({ report_type: "qweb-text", report_name: "stock.report_label" }, {}, testEnv);
        await new Promise((r) => origSetTimeout8(r, 15));
        eq(okTxt, true, "Android text print succeeds");
        eq(sentTextReq.printer, "BT Thermal", "printText passes explicit printer name for Android");
        eq(polledTextIds[0], "android-txt-uuid-7", "text watchJobToCompletion polls Android UUID");
        const canceledRpc = rpcCalls.find((c) => c.url === "/native_print/job_status" && c.params.state === "canceled");
        eq(Boolean(canceledRpc), true, "Android cancelled normalized to canceled");
        eq(canceledRpc.params.idempotency_key, sentTextReq.idempotencyKey, "canceled RPC preserves idempotency_key");
    } finally {
        globalThis.fetch = origFetch8;
        globalThis.setTimeout = origSetTimeout8;
        delete globalThis.window.__NATIVE_PRINT__;
    }

    // 8e. Systray handles Android PrinterInfo (transport, address, protocol: "auto", is_default) & string forms.
    const prevDoc8 = globalThis.document;
    const prevLs8 = globalThis.localStorage;
    globalThis.localStorage = globalThis.window.localStorage;
    globalThis.document = { hidden: false, addEventListener: () => {}, removeEventListener: () => {} };
    try {
        const sMod8 = await import(pathToFileURL(join(tmpDir, "np_systray_test.mjs")).href + "?v8");
        globalThis.__npSystrayRpcs.length = 0;
        globalThis.window.__NATIVE_PRINT__ = {
            appOverview: async () => ({
                printers: [
                    { name: "EPSON TM-T88VII", transport: "bluetooth_spp", protocol: "auto", is_default: true },
                    { name: "Android IPP", transport: "ipp", address: "192.168.1.20", protocol: "pdf" },
                ],
            }),
            printerCapabilities: async () => ({ forms: ["80mm Roll", "A4"], bins: [] }),
        };
        const comp8 = new sMod8.NativePrintSystray();
        comp8.setup();
        await comp8.syncDiscoveredPrinters(false);
        eq(comp8.state.defaultPrinter, "EPSON TM-T88VII", "systray falls back to is_default when defaultPrinter is omitted");
        eq(comp8.state.discoveredPrinters[0].printer_class, "thermal", "systray infers thermal class for protocol: auto");
        eq(comp8.state.discoveredPrinters[0].meta.includes("Bluetooth"), true, "systray formats bluetooth_spp transport");
        eq(comp8.state.discoveredPrinters[0].meta.includes("ESCPOS"), true, "systray resolves auto protocol for meta");
        eq(comp8.state.discoveredPrinters[1].meta.includes("Network"), true, "systray formats ipp transport as Network");
        eq(comp8.state.discoveredPrinters[1].meta.includes("192.168.1.20"), true, "systray displays Android address field");
        const syncDev = globalThis.__npSystrayRpcs.find((c) => c.url === "/native_print/sync_device" && c.params.name === "EPSON TM-T88VII");
        eq(Boolean(syncDev), true, "systray syncs Android printer capabilities");
        eq(syncDev.params.protocol, "escpos", "sync_device sends resolved protocol instead of auto");
        eq(syncDev.params.printer_class, "thermal", "sync_device sends inferred printer_class");
        eq(syncDev.params.forms[0], "80mm Roll", "sync_device preserves Android string forms");
    } finally {
        globalThis.document = prevDoc8;
        globalThis.localStorage = prevLs8;
        delete globalThis.window.__NATIVE_PRINT__;
    }
}

console.log("JS LIVE TARGET TESTS: ALL PASS");

// Node tests for Odoo 20 pos_native_print.js:
//  - posReceiptKey / finishPosReceiptAttempt / extractCashierUserId
//  - watchPosJob & postPosDeclined
//  - PosTicketPrinterPlugin.prototype.printOrderReceipt patch behavior
import { test } from "node:test";
import assert from "node:assert";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const posSrc = readFileSync(
    new URL("../static/src/js/pos_native_print.js", import.meta.url),
    "utf8",
);
const clientSrc = readFileSync(
    new URL("../static/src/js/native_print_client.js", import.meta.url),
    "utf8",
).replace(
    'import { _t } from "@web/core/l10n/translation";',
    "const _t = (s) => s;",
);

function extract(name) {
    let start = posSrc.indexOf(`function ${name}(`);
    assert.ok(start >= 0, `${name} exists`);
    if (start >= 6 && posSrc.slice(start - 6, start) === "async ") start -= 6;
    let depth = 0;
    for (let i = posSrc.indexOf("{", start); i < posSrc.length; i++) {
        if (posSrc[i] === "{") depth++;
        if (posSrc[i] === "}" && --depth === 0) return posSrc.slice(start, i + 1);
    }
    throw new Error(`unbalanced braces in ${name}`);
}

const tmpDir = mkdtempSync(join(tmpdir(), "np-pos20-test-"));
process.on("exit", () => {
    try { rmSync(tmpDir, { recursive: true, force: true }); } catch {}
});
const tmpClient = join(tmpDir, "np_client.mjs");
writeFileSync(tmpClient, clientSrc);
const tmpClientUrl = pathToFileURL(tmpClient).href;
const clientMod = await import(tmpClientUrl);

class StubPosTicketPrinterPlugin {
    async printOrderReceipt(opts) {
        globalThis.__superCalls.push(opts);
        return "super-printed";
    }
}
globalThis.__StubPosTicketPrinterPlugin = StubPosTicketPrinterPlugin;
globalThis.__superCalls = [];
globalThis.__rpcCalls = [];

const runnablePosSrc = posSrc
    .replace(
        'import { PosTicketPrinterPlugin } from "@point_of_sale/app/plugins/pos_ticket_printer_plugin";',
        "const PosTicketPrinterPlugin = globalThis.__StubPosTicketPrinterPlugin;",
    )
    .replace(
        'import { patch } from "@web/core/utils/patch";',
        `function patch(target, ext) {
            const orig = target.printOrderReceipt;
            const holder = { printOrderReceipt: orig };
            Object.setPrototypeOf(ext, holder);
            target.printOrderReceipt = ext.printOrderReceipt;
        }`,
    )
    .replace(
        'import { rpc } from "@web/core/network/rpc";',
        'const rpc = async (url, params) => { globalThis.__rpcCalls.push({ url, params }); return { ok: true }; };',
    )
    .replace(
        'import { _t } from "@web/core/l10n/translation";',
        'const _t = (s, ...args) => args.reduce((acc, v) => acc.replace("%s", String(v)), s);',
    )
    .replace('from "./native_print_client";', `from "${tmpClientUrl}";`);

const tmpPos = join(tmpDir, "pos_native_print_test.mjs");
writeFileSync(tmpPos, runnablePosSrc);
const posMod = await import(pathToFileURL(tmpPos).href);

test("document direction follows session language: Arabic rtl, English ltr", () => {
    const prevDocument = globalThis.document;
    const listeners = [];
    globalThis.document = {
        documentElement: { lang: "ar_001", dir: "ltr" },
        addEventListener: (type, fn) => listeners.push({ type, fn }),
    };
    try {
        assert.equal(clientMod.directionForLang("ar"), "rtl");
        assert.equal(clientMod.directionForLang("ar_001"), "rtl");
        assert.equal(clientMod.directionForLang("he_IL"), "rtl");
        assert.equal(clientMod.directionForLang("en_US"), "ltr");

        assert.equal(clientMod.syncDocumentDirection(), "rtl");
        assert.equal(globalThis.document.documentElement.dir, "rtl", "Arabic must render rtl");

        globalThis.document.documentElement.lang = "en_US";
        assert.equal(clientMod.syncDocumentDirection(), "ltr", "English must render ltr");
        assert.equal(globalThis.document.documentElement.dir, "ltr");
    } finally {
        globalThis.document = prevDocument;
    }
});

test("posReceiptKey idempotency, per-cashier scoping, tombstone rotation, and stale >15s expiry", () => {
    const store = new Map();
    const prevStorage = globalThis.sessionStorage;
    globalThis.sessionStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
    };
    try {
        eval(
            extract("extractCashierUserId") + "\n" +
            extract("posReceiptKey") + "\n" +
            extract("finishPosReceiptAttempt") +
            "\nglobalThis.__u = extractCashierUserId; globalThis.__k = posReceiptKey; globalThis.__c = finishPosReceiptAttempt;",
        );

        const k1 = globalThis.__k("Order_1", "7");
        const k2 = globalThis.__k("Order_1", "7");
        assert.strictEqual(k1, k2, "double-fire reuses the in-flight key");
        assert.match(k1, /^pos-Order_1-\d{8}-u7-a1$/);

        globalThis.__c("Order_1", "7");
        const k3 = globalThis.__k("Order_1", "7");
        assert.match(k3, /-a2$/, "reprint after terminal is a fresh attempt");
        assert.notStrictEqual(k3, k1);

        assert.notStrictEqual(globalThis.__k("Order_2", "7"), globalThis.__k("Order_1", "7"));

        const other = globalThis.__k("Order_1", "9");
        assert.match(other, /^pos-Order_1-\d{8}-u9-a1$/);
        assert.notStrictEqual(other, k3);
        assert.strictEqual(globalThis.__k("Order_1", "9"), other, "second cashier keeps own in-flight key");
        assert.strictEqual(globalThis.__k("Order_1", "7"), k3, "first cashier record untouched");

        store.set("np.pos.key.Order_3.u7", JSON.stringify({
            key: "pos-Order_3-20261119-u7-a1",
            ts: Date.now() - 20000,
            attempt: 1,
        }));
        const staleNext = globalThis.__k("Order_3", "7");
        assert.match(staleNext, /-a2$/, "stale >15s record advances to next attempt");

        assert.strictEqual(globalThis.__u({ name: "Anon" }, { user: { id: 12 } }, null), "12");
        assert.strictEqual(globalThis.__u({}, { user: {}, session: { user_id: [19, "Cashier"] } }, null), "19");
        assert.strictEqual(globalThis.__u(null, null, { user_id: { id: 42 } }), "42");
    } finally {
        globalThis.sessionStorage = prevStorage;
    }
});

test("watchPosJob and postPosDeclined sync job status, dispatch events, and sanitize errors", async () => {
    const rpcCalls = [];
    const posNotices = [];
    let jobUpdatedEvents = 0;
    const prevWindow = globalThis.window;
    const prevSetTimeout = globalThis.setTimeout;
    const prevStorage = globalThis.sessionStorage;
    const store = new Map();
    globalThis.sessionStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
    };
    globalThis.window = {
        dispatchEvent(ev) {
            if (ev && ev.type === "native_print:job_updated") jobUpdatedEvents++;
            return true;
        },
    };
    globalThis.setTimeout = (fn) => { fn(); return 0; };
    try {
        globalThis.__posRpcCalls = rpcCalls;
        globalThis.__posNotices = posNotices;
        eval([
            'const POS_RECEIPT_SOURCE = "point_of_sale.receipt";',
            'const TERMINAL_STATES = new Set(["done", "failed", "canceled", "dead_letter"]);',
            'const _t = (s, ...args) => args.reduce((acc, v) => acc.replace("%s", String(v)), s);',
            'const notifyUser = (_env, msg, type) => { globalThis.__posNotices.push({ msg, type }); };',
            'const safeErrorCode = (code, fallback = "error") => { const t = typeof code === "string" ? code.trim() : ""; return /^[A-Za-z0-9_.-]{1,64}$/.test(t) ? t : fallback; };',
            'const notifyJobUpdated = () => { if (typeof window !== "undefined" && typeof window.dispatchEvent === "function") window.dispatchEvent({ type: "native_print:job_updated" }); };',
            'const rpc = async (url, params) => { globalThis.__posRpcCalls.push({ url, params }); return { ok: true }; };',
            extract("finishPosReceiptAttempt"),
            extract("watchPosJob"),
            extract("postPosDeclined"),
            "globalThis.__watchPosJob = watchPosJob; globalThis.__postPosDeclined = postPosDeclined;",
        ].join("\n"));

        const order = { _printed: false };
        await globalThis.__watchPosJob(
            { jobDetail: async () => ({ state: "done", attempts: 1 }) },
            "pos-ev-1",
            { printer_pattern: "EPSON", copies: 1, mode: "escpos" },
            128,
            "Order_Ev1",
            "EPSON",
            null,
            "7",
            order,
        );
        assert.strictEqual(order._printed, true, "watchPosJob marks order printed on done");
        assert.strictEqual(jobUpdatedEvents, 1, "watchPosJob dispatches native_print:job_updated on done");

        globalThis.__postPosDeclined("pos-decl-1", "POS_NO_PRINTER", "Order_Ev2", "7", "EPSON", "escpos");
        await new Promise((r) => prevSetTimeout(r, 10));
        assert.strictEqual(jobUpdatedEvents, 2, "postPosDeclined dispatches native_print:job_updated");

        const failOrder = { _printed: true };
        await globalThis.__watchPosJob(
            { jobDetail: async () => ({ state: "dead_letter", errorCode: "Error: SECRET_TRACE\n at rust.rs:42", attempts: 3 }) },
            "pos-ev-fail",
            { printer_pattern: "EPSON", copies: 1, mode: "escpos" },
            128,
            "Order_EvFail",
            "EPSON",
            null,
            "7",
            failOrder,
        );
        assert.strictEqual(failOrder._printed, false, "watchPosJob clears _printed on dead_letter");
        assert.strictEqual(jobUpdatedEvents, 3, "watchPosJob dispatches native_print:job_updated on dead_letter");
        assert.strictEqual(posNotices.length, 1, "watchPosJob notifies user once on dead_letter");
        assert.strictEqual(posNotices[0].type, "danger");
        assert.ok(posNotices[0].msg.includes("(dead_letter)"));
        assert.ok(!posNotices[0].msg.includes("SECRET_TRACE"));
    } finally {
        globalThis.window = prevWindow;
        globalThis.setTimeout = prevSetTimeout;
        globalThis.sessionStorage = prevStorage;
        delete globalThis.__posRpcCalls;
        delete globalThis.__posNotices;
    }
});

test("PosTicketPrinterPlugin.printOrderReceipt delegates to super when outside native app", async () => {
    const prevWindow = globalThis.window;
    globalThis.window = {};
    globalThis.__superCalls.length = 0;
    try {
        const plugin = new StubPosTicketPrinterPlugin();
        const res = await plugin.printOrderReceipt({ order: { uuid: "o-browser" } });
        assert.strictEqual(res, "super-printed");
        assert.strictEqual(globalThis.__superCalls.length, 1);
    } finally {
        globalThis.window = prevWindow;
    }
});

test("PosTicketPrinterPlugin.printOrderReceipt prints image natively, marks printed, and cleans up iframe", async () => {
    const prevWindow = globalThis.window;
    const prevStorage = globalThis.sessionStorage;
    const prevSetTimeout = globalThis.setTimeout;
    const store = new Map();
    const printImageCalls = [];
    const markedOrders = [];
    const iframeCalls = [];
    let iframeRemoved = false;

    globalThis.sessionStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
    };
    globalThis.setTimeout = (fn, ms) => (ms <= 2000 ? (fn(), 0) : prevSetTimeout(fn, ms));
    globalThis.window = {
        localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
        dispatchEvent: () => true,
        __NATIVE_PRINT__: {
            listPrinters: async () => [
                { name: "EPSON TM-T88VII", driver: "EPSON", kind: "network", port: "192.168.1.50", is_default: true },
            ],
            printerCapabilities: async () => ({ forms: [{ roll: true, size: { width_mm: 80 } }], color: false }),
            printImage: async (req) => { printImageCalls.push(req); return req.idempotencyKey; },
            jobDetail: async () => ({ state: "done", attempts: 1 }),
        },
    };
    globalThis.__rpcCalls.length = 0;

    try {
        const order = { uuid: "ord-20-img", _printed: false };
        const plugin = new StubPosTicketPrinterPlugin();
        plugin.pos = {
            getCashier: () => ({ id: 15 }),
            getOrder: () => order,
        };
        plugin.notification = { add() {} };
        plugin.generateIframe = async (...args) => {
            iframeCalls.push(args);
            return {
                contentDocument: {
                    getElementById: (id) => (id === "pos-receipt" ? { offsetWidth: 300, innerText: "Receipt #1" } : null),
                },
                remove: () => { iframeRemoved = true; },
            };
        };
        plugin.generateImage = async () => ({
            width: 300,
            toBlob(cb) {
                cb({ arrayBuffer: async () => new Uint8Array([137, 80, 78, 71]).buffer });
            },
        });
        plugin.markReceiptAsPrinted = async (o) => { markedOrders.push(o); };

        const ok = await plugin.printOrderReceipt({ order, basic: false, printBillActionTriggered: false });
        assert.strictEqual(ok, true);
        assert.strictEqual(iframeCalls[0][0], "point_of_sale.pos_order_receipt");
        assert.strictEqual(iframeRemoved, true, "iframe cleaned up after capture");
        assert.strictEqual(printImageCalls.length, 1);
        assert.strictEqual(printImageCalls[0].printerPattern, "tcp://192.168.1.50:9100");
        assert.strictEqual(printImageCalls[0].mode, "escpos");
        assert.deepStrictEqual(printImageCalls[0].bytes, [137, 80, 78, 71]);
        assert.match(printImageCalls[0].idempotencyKey, /^pos-ord-20-img-\d{8}-u15-a1$/);
        assert.strictEqual(order._printed, true);
        assert.deepStrictEqual(markedOrders, [order]);
    } finally {
        globalThis.window = prevWindow;
        globalThis.sessionStorage = prevStorage;
        globalThis.setTimeout = prevSetTimeout;
    }
});

test("PosTicketPrinterPlugin.printOrderReceipt falls back to ESC/POS text when generateImage fails and respects printBillActionTriggered", async () => {
    const prevWindow = globalThis.window;
    const prevStorage = globalThis.sessionStorage;
    const prevSetTimeout = globalThis.setTimeout;
    const store = new Map();
    const printTextCalls = [];
    const markedOrders = [];

    globalThis.sessionStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
    };
    globalThis.setTimeout = (fn, ms) => (ms <= 2000 ? (fn(), 0) : prevSetTimeout(fn, ms));
    globalThis.window = {
        localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
        dispatchEvent: () => true,
        __NATIVE_PRINT__: {
            listPrinters: async () => [
                { name: "EPSON TM-T88VII", driver: "EPSON", kind: "local", is_default: true },
            ],
            printerCapabilities: async () => ({ forms: [], color: false }),
            printImage: async () => { throw new Error("should not call printImage"); },
            printText: async (req) => { printTextCalls.push(req); return req.idempotencyKey; },
            jobDetail: async () => ({ state: "done", attempts: 1 }),
        },
    };

    try {
        const order = { uuid: "ord-20-txt", _printed: false };
        const plugin = new StubPosTicketPrinterPlugin();
        plugin.pos = { get_cashier: () => ({ id: 21 }) };
        plugin.generateIframe = async () => ({
            contentDocument: {
                getElementById: (id) => (id === "pos-receipt" ? { innerText: "TOTAL 42.00\nTHANK YOU" } : null),
            },
            remove() {},
        });
        plugin.generateImage = async () => { throw new Error("canvas failed"); };
        plugin.markReceiptAsPrinted = async (o) => { markedOrders.push(o); };

        const ok = await plugin.printOrderReceipt({ order, printBillActionTriggered: true });
        assert.strictEqual(ok, true);
        assert.strictEqual(printTextCalls.length, 1);
        assert.strictEqual(printTextCalls[0].text, "TOTAL 42.00\nTHANK YOU");
        assert.strictEqual(printTextCalls[0].cut, true);
        assert.strictEqual(markedOrders.length, 0, "bill print must not call markReceiptAsPrinted");
    } finally {
        globalThis.window = prevWindow;
        globalThis.sessionStorage = prevStorage;
        globalThis.setTimeout = prevSetTimeout;
    }
});

test("PosTicketPrinterPlugin.printOrderReceipt notifies via NotificationPlugin when no printer is available", async () => {
    const prevWindow = globalThis.window;
    const prevStorage = globalThis.sessionStorage;
    const prevSetTimeout = globalThis.setTimeout;
    const store = new Map();
    const notices = [];

    globalThis.sessionStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
    };
    globalThis.setTimeout = (fn, ms) => (ms <= 2000 ? (fn(), 0) : prevSetTimeout(fn, ms));
    globalThis.window = {
        localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
        dispatchEvent: () => true,
        __NATIVE_PRINT__: {
            listPrinters: async () => [],
            printImage: async () => "never",
        },
    };

    try {
        const plugin = new StubPosTicketPrinterPlugin();
        plugin.pos = { getCashier: () => ({ id: 8 }) };
        plugin.notification = { add: (msg, opts) => notices.push({ msg, opts }) };

        const ok = await plugin.printOrderReceipt({ order: { uuid: "ord-20-noprn" } });
        assert.strictEqual(ok, true);
        assert.strictEqual(notices.length, 1);
        assert.strictEqual(notices[0].opts.type, "warning");
        assert.ok(notices[0].msg.includes("no printer is available"));
    } finally {
        globalThis.window = prevWindow;
        globalThis.sessionStorage = prevStorage;
        globalThis.setTimeout = prevSetTimeout;
    }
});

test("PosTicketPrinterPlugin.printOrderReceipt handles generateIframe rejection cleanly without double-calling", async () => {
    const prevWindow = globalThis.window;
    const prevStorage = globalThis.sessionStorage;
    const prevSetTimeout = globalThis.setTimeout;
    const store = new Map();
    const notices = [];
    let iframeCalls = 0;

    globalThis.sessionStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
    };
    globalThis.setTimeout = (fn, ms) => (ms <= 2000 ? (fn(), 0) : prevSetTimeout(fn, ms));
    globalThis.window = {
        localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
        dispatchEvent: () => true,
        __NATIVE_PRINT__: {
            listPrinters: async () => [{ name: "Star TSP100", printer_class: "thermal", protocol: "gdi", online: true }],
            printImage: async () => { throw new Error("should not print"); },
            printText: async () => { throw new Error("should not print"); },
        },
    };

    try {
        const plugin = new StubPosTicketPrinterPlugin();
        plugin.pos = { getCashier: () => ({ id: 7 }) };
        plugin.notification = { add: (msg, opts) => notices.push({ msg, opts }) };
        plugin.generateIframe = async (_tpl, _props) => {
            iframeCalls += 1;
            throw new Error("image load timeout");
        };
        plugin.generateImage = async () => { throw new Error("should not run"); };

        const ok = await plugin.printOrderReceipt({ order: { uuid: "ord-20-iframe-err", name: "Order 99" } });
        assert.strictEqual(ok, true);
        assert.strictEqual(iframeCalls, 1, "must not retry generateIframe with wrong arity on runtime error");
        assert.strictEqual(notices.length, 1);
        assert.strictEqual(notices[0].opts.type, "warning");
    } finally {
        globalThis.window = prevWindow;
        globalThis.sessionStorage = prevStorage;
        globalThis.setTimeout = prevSetTimeout;
    }
});

test("PosTicketPrinterPlugin.printOrderReceipt logs POS_ENQUEUE_FAILED on primitive/null bridge rejection and dedupes concurrent clicks", async () => {
    const prevWindow = globalThis.window;
    const prevStorage = globalThis.sessionStorage;
    const prevSetTimeout = globalThis.setTimeout;
    const store = new Map();
    const notices = [];
    let markedCount = 0;

    globalThis.sessionStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
    };
    globalThis.setTimeout = (fn, ms) => (ms <= 2000 ? (fn(), 0) : prevSetTimeout(fn, ms));
    globalThis.__rpcCalls.length = 0;
    globalThis.window = {
        localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
        dispatchEvent: () => true,
        __NATIVE_PRINT__: {
            listPrinters: async () => [{ name: "EPSON TM-T88VII", driver: "EPSON", kind: "local", is_default: true }],
            printerCapabilities: async () => ({ forms: [], color: false }),
            printImage: async () => { throw "spooler busy"; },
        },
    };

    try {
        const plugin = new StubPosTicketPrinterPlugin();
        plugin.pos = { getCashier: () => ({ id: 33 }) };
        plugin.notification = { add: (msg, opts) => notices.push({ msg, opts }) };
        plugin.generateIframe = async () => ({
            contentDocument: { getElementById: () => ({ offsetWidth: 300, innerText: "Receipt" }) },
            remove() {},
        });
        plugin.generateImage = async () => ({
            width: 300,
            toBlob(cb) { cb({ arrayBuffer: async () => new Uint8Array([137, 80, 78, 71]).buffer }); },
        });
        plugin.markReceiptAsPrinted = async () => { markedCount += 1; };

        await plugin.printOrderReceipt({ order: { uuid: "ord-20-str-err" } });
        assert.strictEqual(notices.length, 1);
        assert.strictEqual(
            globalThis.__rpcCalls.some((c) => c.url === "/native_print/job_status" && c.params.error_code === "POS_ENQUEUE_FAILED"),
            true,
            "primitive string rejection must still log POS_ENQUEUE_FAILED",
        );
        assert.strictEqual(markedCount, 0, "failed enqueue must not mark receipt printed");

        // Concurrent double-click while printImage is pending must only call markReceiptAsPrinted once.
        let releasePrint;
        globalThis.window.__NATIVE_PRINT__.printImage = () => new Promise((resolve) => { releasePrint = resolve; });
        globalThis.window.__NATIVE_PRINT__.jobDetail = async () => ({ state: "done", attempts: 1 });
        const order = { uuid: "ord-20-dedupe", _printed: false };
        const p1 = plugin.printOrderReceipt({ order });
        await new Promise((r) => prevSetTimeout(r, 5));
        const p2 = await plugin.printOrderReceipt({ order });
        assert.strictEqual(p2, true);
        assert.strictEqual(markedCount, 0, "deduped second click must not prematurely mark receipt printed");
        releasePrint("ok");
        await p1;
        assert.strictEqual(markedCount, 1, "only the active print marks receipt printed once");
    } finally {
        globalThis.window = prevWindow;
        globalThis.sessionStorage = prevStorage;
        globalThis.setTimeout = prevSetTimeout;
    }
});

test("PosTicketPrinterPlugin.printOrderReceipt cleans up late iframe on generateIframe timeout and falls back on generateImage timeout", async () => {
    const prevWindow = globalThis.window;
    const prevStorage = globalThis.sessionStorage;
    const prevSetTimeout = globalThis.setTimeout;
    const prevClearTimeout = globalThis.clearTimeout;
    const prevDateNow = Date.now;
    Date.now = () => prevDateNow() + 10_000;
    const store = new Map();
    const notices = [];
    const printTextCalls = [];
    let clearedCaptureTimers = 0;

    globalThis.sessionStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
    };
    globalThis.__rpcCalls.length = 0;
    globalThis.setTimeout = (fn, ms) => {
        if (ms === posMod.POS_CAPTURE_TIMEOUT_MS) {
            queueMicrotask(fn);
            return 9001;
        }
        if (ms <= 2000) { fn(); return 0; }
        return prevSetTimeout(fn, ms);
    };
    globalThis.clearTimeout = (id) => {
        if (id === 9001) clearedCaptureTimers += 1;
        return prevClearTimeout(id);
    };
    globalThis.window = {
        localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
        dispatchEvent: () => true,
        __NATIVE_PRINT__: {
            listPrinters: async () => [{ name: "EPSON TM-T88VII", driver: "EPSON", kind: "local", is_default: true }],
            printerCapabilities: async () => ({ forms: [], color: false }),
            printImage: async () => { throw new Error("should not call printImage"); },
            printText: async (req) => { printTextCalls.push(req); return req.idempotencyKey; },
            jobDetail: async () => ({ state: "done", attempts: 1 }),
        },
    };

    try {
        // 1. Hung generateIframe times out, logs POS_CAPTURE_FAILED, and removes late-resolving iframe.
        let resolveLateIframe;
        let lateIframeRemoved = false;
        const plugin = new StubPosTicketPrinterPlugin();
        plugin.pos = { getCashier: () => ({ id: 41 }) };
        plugin.notification = { add: (msg, opts) => notices.push({ msg, opts }) };
        plugin.generateIframe = () => new Promise((resolve) => { resolveLateIframe = resolve; });
        plugin.generateImage = async () => { throw new Error("should not run"); };

        const ok1 = await plugin.printOrderReceipt({ order: { uuid: "ord-20-iframe-timeout" } });
        assert.strictEqual(ok1, true);
        assert.strictEqual(notices.length, 1);
        assert.strictEqual(
            globalThis.__rpcCalls.some((c) => c.url === "/native_print/job_status" && c.params.error_code === "POS_CAPTURE_FAILED"),
            true,
            "generateIframe timeout logs POS_CAPTURE_FAILED",
        );
        resolveLateIframe({ remove: () => { lateIframeRemoved = true; } });
        await new Promise((r) => prevSetTimeout(r, 10));
        assert.strictEqual(lateIframeRemoved, true, "late-resolving iframe is removed after timeout");

        // 2. Hung generateImage times out, cleans up iframe in finally, and falls back to ESC/POS printText.
        let imgIframeRemoved = false;
        let captureTimerCount = 0;
        globalThis.setTimeout = (fn, ms) => {
            if (ms === posMod.POS_CAPTURE_TIMEOUT_MS) {
                captureTimerCount += 1;
                if (captureTimerCount === 2) {
                    queueMicrotask(fn);
                }
                return 9001;
            }
            if (ms <= 2000) { fn(); return 0; }
            return prevSetTimeout(fn, ms);
        };
        plugin.generateIframe = async () => ({
            contentDocument: {
                getElementById: (id) => (id === "pos-receipt" ? { innerText: "FALLBACK RECEIPT" } : null),
            },
            remove: () => { imgIframeRemoved = true; },
        });
        plugin.generateImage = () => new Promise(() => {});

        const ok2 = await plugin.printOrderReceipt({ order: { uuid: "ord-20-img-timeout" } });
        assert.strictEqual(ok2, true);
        assert.strictEqual(imgIframeRemoved, true, "iframe is removed when generateImage times out");
        assert.strictEqual(printTextCalls.length, 1, "generateImage timeout falls back to ESC/POS printText");
        assert.strictEqual(printTextCalls[0].text, "FALLBACK RECEIPT");
        assert.ok(clearedCaptureTimers >= 1, "capture timeout timer is cleared on normal settle");
    } finally {
        globalThis.window = prevWindow;
        globalThis.sessionStorage = prevStorage;
        globalThis.setTimeout = prevSetTimeout;
        globalThis.clearTimeout = prevClearTimeout;
        Date.now = prevDateNow;
    }
});

test("OneNote-style GDI printer: failed image capture prints text, total capture failure reports natively (no print dialog)", async () => {
    const prevWindow = globalThis.window;
    const prevStorage = globalThis.sessionStorage;
    const prevSetTimeout = globalThis.setTimeout;
    const prevSuperCount = globalThis.__superCalls.length;
    const store = new Map();
    const printTextCalls = [];
    const notices = [];

    globalThis.sessionStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
    };
    globalThis.setTimeout = (fn, ms) => (ms <= 2000 ? (fn(), 0) : prevSetTimeout(fn, ms));
    globalThis.window = {
        localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
        dispatchEvent: () => true,
        __NATIVE_PRINT__: {
            listPrinters: async () => [
                { name: "OneNote (Desktop)", printer_class: "office", protocol: "gdi", online: true },
            ],
            printerCapabilities: async () => ({ forms: [], color: true }),
            printImage: async () => { throw new Error("must not print an image"); },
            printText: async (req) => { printTextCalls.push(req); return req.idempotencyKey; },
            jobDetail: async () => ({ state: "done", attempts: 1 }),
        },
    };

    try {
        // 1) A GDI printer must still get the receipt text when the image
        //    capture path fails (previously only ESC/POS had this fallback).
        const plugin = new StubPosTicketPrinterPlugin();
        plugin.pos = { getCashier: () => ({ id: 31 }) };
        plugin.notification = { add: (msg, opts) => notices.push({ msg, opts }) };
        plugin.generateIframe = async () => ({
            contentDocument: {
                getElementById: () => ({ innerText: "TOTAL 10.00", offsetWidth: 300 }),
            },
            remove() {},
        });
        plugin.generateImage = async () => { throw new Error("GDI capture failed"); };

        assert.strictEqual(
            await plugin.printOrderReceipt({ order: { uuid: "ord-20-onenote" } }),
            true,
        );
        assert.strictEqual(printTextCalls.length, 1, "GDI capture failure must fall back to printText");
        assert.strictEqual(printTextCalls[0].text, "TOTAL 10.00");
        assert.strictEqual(globalThis.__superCalls.length, prevSuperCount, "text fallback must not use the browser");

        // 2) If neither image nor text can be produced, fail loudly but
        //    never open Odoo's browser print dialog.
        const plugin2 = new StubPosTicketPrinterPlugin();
        plugin2.pos = { getCashier: () => ({ id: 32 }) };
        plugin2.notification = { add: (msg, opts) => notices.push({ msg, opts }) };
        plugin2.generateIframe = async () => { throw new Error("template unavailable"); };
        plugin2.generateImage = async () => { throw new Error("should not run"); };

        assert.strictEqual(
            await plugin2.printOrderReceipt({ order: { uuid: "ord-20-onenote-2" } }),
            true,
        );
        assert.strictEqual(
            globalThis.__superCalls.length,
            prevSuperCount,
            "capture failure must never open Odoo's browser print dialog",
        );
        assert.ok(
            globalThis.__rpcCalls.some(
                (c) => c.url === "/native_print/job_status" && c.params.error_code === "POS_CAPTURE_FAILED",
            ),
            "the native failure is still recorded for diagnostics",
        );
    } finally {
        globalThis.window = prevWindow;
        globalThis.sessionStorage = prevStorage;
        globalThis.setTimeout = prevSetTimeout;
    }
});

test("Odoo 20 builds POS receipt data before rendering the QWeb template", async () => {
    const prevWindow = globalThis.window;
    const prevStorage = globalThis.sessionStorage;
    const prevSetTimeout = globalThis.setTimeout;
    const store = new Map();
    const printImageCalls = [];
    const generatedData = { company: { name: "Test" }, order: {}, lines: [] };
    let dataArgs = null;
    let iframeArgs = null;

    globalThis.sessionStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
    };
    globalThis.setTimeout = (fn, ms) => (ms <= 2000 ? (fn(), 0) : prevSetTimeout(fn, ms));
    globalThis.window = {
        localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
        dispatchEvent: () => true,
        __NATIVE_PRINT__: {
            listPrinters: async () => [
                { name: "OneNote (Desktop)", printer_class: "office", protocol: "gdi", online: true },
            ],
            printerCapabilities: async () => ({ forms: [], color: true }),
            printImage: async (req) => { printImageCalls.push(req); return req.idempotencyKey; },
            printText: async () => { throw new Error("should not print text"); },
            jobDetail: async () => ({ state: "done", attempts: 1 }),
        },
    };

    try {
        const order = { uuid: "ord-20-data" };
        const plugin = new StubPosTicketPrinterPlugin();
        plugin.pos = { getCashier: () => ({ id: 41 }) };
        plugin.notification = { add() {} };
        plugin.getOrderReceiptData = async (o, opts) => {
            dataArgs = { o, opts };
            return generatedData;
        };
        plugin.generateIframe = async (...args) => {
            iframeArgs = args;
            return {
                contentDocument: { getElementById: () => ({ offsetWidth: 300 }) },
                remove() {},
            };
        };
        plugin.generateImage = async () => ({
            width: 300,
            toBlob(cb) {
                cb({ arrayBuffer: async () => new Uint8Array([137, 80, 78, 71]).buffer });
            },
        });

        assert.strictEqual(
            await plugin.printOrderReceipt({ order, basic: true, simplified: false }),
            true,
        );
        assert.strictEqual(dataArgs.o, order, "POS models must receive the active order");
        assert.deepStrictEqual(
            dataArgs.opts,
            { basic: true, simplified: false },
            "basic/simplified flags must be forwarded",
        );
        assert.strictEqual(iframeArgs[0], "point_of_sale.pos_order_receipt");
        assert.strictEqual(
            iframeArgs[1],
            generatedData,
            "the generated data, not raw props, must render the template",
        );
        assert.strictEqual(printImageCalls.length, 1, "the captured PNG must be sent to the printer");
    } finally {
        globalThis.window = prevWindow;
        globalThis.sessionStorage = prevStorage;
        globalThis.setTimeout = prevSetTimeout;
    }
});

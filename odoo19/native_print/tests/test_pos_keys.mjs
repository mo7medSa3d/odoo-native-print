// POS receipt idempotency keys: extracted from pos_native_print.js and
// exercised against a sessionStorage stub.
//  - same order resubmitted within 15s -> SAME key (desktop dedupes)
//  - after the watcher clears the record (terminal state) -> next attempt
//  - a stale record (>15s, no terminal seen) -> next attempt, never a reuse
import fs from "node:fs";
import assert from "node:assert";

const src = fs.readFileSync(
    new URL("../static/src/js/pos_native_print.js", import.meta.url), "utf8");
function extract(name) {
    let start = src.indexOf(`function ${name}(`);
    assert.ok(start >= 0, `${name} exists`);
    if (start >= 6 && src.slice(start - 6, start) === "async ") start -= 6;
    let depth = 0;
    for (let i = src.indexOf("{", start); i < src.length; i++) {
        if (src[i] === "{") depth++;
        if (src[i] === "}" && --depth === 0) return src.slice(start, i + 1);
    }
    throw new Error("unbalanced");
}
function makeWebStorage(map = new Map()) {
    return {
        get length() { return map.size; },
        key: (i) => Array.from(map.keys())[i] ?? null,
        getItem: (k) => (map.has(k) ? map.get(k) : null),
        setItem: (k, v) => map.set(k, String(v)),
        removeItem: (k) => map.delete(k),
    };
}
const localStore = new Map();
let sessionStore = new Map();
globalThis.localStorage = makeWebStorage(localStore);
globalThis.sessionStorage = makeWebStorage(sessionStore);
eval(extract("extractCashierUserId") + "\n" + extract("posReceiptKey") + "\n" + extract("finishPosReceiptAttempt") +
    "\nglobalThis.__u = extractCashierUserId; globalThis.__k = posReceiptKey; globalThis.__c = finishPosReceiptAttempt;");

const k1 = globalThis.__k("Order_1", "7");
const k2 = globalThis.__k("Order_1", "7");
assert.strictEqual(k1, k2, "double-fire reuses the in-flight key");
assert.match(k1, /^pos-Order_1-\d{8}-u7-a1$/);
globalThis.__c("Order_1", "7"); // watcher saw a terminal state
const k3 = globalThis.__k("Order_1", "7");
assert.match(k3, /-a2$/, "reprint after terminal is a fresh attempt");
assert.notStrictEqual(k3, k1);
// Different orders never share a key.
assert.notStrictEqual(globalThis.__k("Order_2", "7"), globalThis.__k("Order_1", "7"));
// Per-cashier scoping: the SAME order printed by a DIFFERENT cashier is a
// different job — different key, and its own in-flight record (the
// first cashier's record must not dedupe or clear it).
const other = globalThis.__k("Order_1", "9");
assert.match(other, /^pos-Order_1-\d{8}-u9-a1$/);
assert.notStrictEqual(other, k3);
assert.strictEqual(globalThis.__k("Order_1", "9"), other, "second cashier keeps own in-flight key");
assert.strictEqual(globalThis.__k("Order_1", "7"), k3, "first cashier record untouched");
// Stale record (>15s without terminal state) advances to a fresh attempt.
const today = new Date().toISOString().slice(0, 10).replace(/-/g, "");
localStore.set(`np.pos.key.${today}.Order_3.u7`, JSON.stringify({ key: `pos-Order_3-${today}-u7-a1`, ts: Date.now() - 20000, attempt: 1 }));
const staleNext = globalThis.__k("Order_3", "7");
assert.match(staleNext, /-a2$/, "stale >15s record advances to next attempt");

// Persistence across simulated tab reload: clearing sessionStorage keeps localStorage tombstone.
const r1 = globalThis.__k("Order_Reload", "7");
assert.match(r1, /-a1$/);
globalThis.__c("Order_Reload", "7");
sessionStore = new Map();
globalThis.sessionStorage = makeWebStorage(sessionStore);
const r2 = globalThis.__k("Order_Reload", "7");
assert.match(r2, /-a2$/, "reprint after tab reload advances to a2 instead of colliding with a1");
assert.notStrictEqual(r2, r1);

// Pruning: entries older than 2 days are removed on write; recent entries are preserved.
const oldDay = new Date(Date.now() - 3 * 86400000).toISOString().slice(0, 10).replace(/-/g, "");
const keepDay = new Date(Date.now() - 1 * 86400000).toISOString().slice(0, 10).replace(/-/g, "");
localStore.set(`np.pos.key.${oldDay}.Order_Old.u7`, JSON.stringify({ key: null, ts: 0, attempt: 1 }));
localStore.set(`np.pos.key.${keepDay}.Order_Keep.u7`, JSON.stringify({ key: null, ts: 0, attempt: 1 }));
globalThis.__k("Order_PruneTrigger", "7");
assert.strictEqual(localStore.has(`np.pos.key.${oldDay}.Order_Old.u7`), false, "entries older than 2 days are pruned on write");
assert.strictEqual(localStore.has(`np.pos.key.${keepDay}.Order_Keep.u7`), true, "entries within 2 days are kept");

// Fallback to sessionStorage when localStorage throws.
const prevLsFallback = globalThis.localStorage;
globalThis.localStorage = {
    getItem() { throw new Error("SecurityError"); },
    setItem() { throw new Error("SecurityError"); },
    removeItem() { throw new Error("SecurityError"); },
};
try {
    const fb1 = globalThis.__k("Order_Fallback", "7");
    assert.match(fb1, /-a1$/);
    globalThis.__c("Order_Fallback", "7");
    const fb2 = globalThis.__k("Order_Fallback", "7");
    assert.match(fb2, /-a2$/, "falls back to sessionStorage when localStorage throws");
} finally {
    globalThis.localStorage = prevLsFallback;
}

// Cashier ID extraction: an object without .id falls through instead of returning "0".
assert.strictEqual(
    globalThis.__u({ name: "Anon" }, { user: { id: 12 } }, null),
    "12",
    "cashier object without id falls through to posStore.user.id",
);
assert.strictEqual(
    globalThis.__u({}, { user: {}, session: { user_id: [19, "Cashier"] } }, null),
    "19",
    "empty cashier/user objects fall through to session.user_id tuple",
);
assert.strictEqual(
    globalThis.__u(null, null, { user_id: { id: 42 } }),
    "42",
    "falls through to order.user_id.id",
);

// Regression test: single-pass getComputedStyle, root backgroundImage inlining,
// URL fetch deduplication, and timeout/error fallback in inlineStyles + inlineMedia.
{
    const helpers = [
        "const MEDIA_FETCH_TIMEOUT_MS = 8000;",
        'const TRANSPARENT_PIXEL = "data:image/png;base64,TRANSPARENT";',
        (src.match(/const RECEIPT_LAYOUT_FIX_CSS = [\s\S]*?;/)?.[0] || 'const RECEIPT_LAYOUT_FIX_CSS = "";'),
        extract("inlineStyles"),
        extract("fetchAsDataUrl"),
        extract("inlineMedia"),
        "globalThis.__inlineStyles = inlineStyles; globalThis.__inlineMedia = inlineMedia;",
    ].join("\n");
    eval(helpers);

    const makeNode = (tag, { attrs = {}, bg = "none", cssText = "color:black;", children = [] } = {}) => ({
        tag,
        _attrs: { ...attrs },
        _bg: bg,
        _cssText: cssText,
        style: {},
        children,
        get src() { return this._attrs.src || ""; },
        get currentSrc() { return this._attrs.src || ""; },
        setAttribute(k, v) { this._attrs[k] = v; },
        getAttribute(k) { return this._attrs[k] ?? null; },
        querySelectorAll(sel) {
            const out = [];
            const walk = (n) => {
                for (const c of n.children) {
                    if (c.tag === sel) out.push(c);
                    walk(c);
                }
            };
            walk(this);
            return out;
        },
    });

    const srcImg1 = makeNode("img", { attrs: { src: "https://shop.test/logo.png" } });
    const srcImg2 = makeNode("img", { attrs: { src: "https://shop.test/logo.png" } });
    const srcDiv = makeNode("div", { bg: 'url("https://shop.test/broken.png")' });
    const srcRoot = makeNode("div", { bg: 'url("https://shop.test/bg.png")', children: [srcImg1, srcImg2, srcDiv] });

    const clImg1 = makeNode("img", { attrs: { src: "https://shop.test/logo.png" } });
    const clImg2 = makeNode("img", { attrs: { src: "https://shop.test/logo.png" } });
    const clDiv = makeNode("div", {});
    const clRoot = makeNode("div", { children: [clImg1, clImg2, clDiv] });

    let computedCalls = 0;
    const prevWindow = globalThis.window;
    const prevFetch = globalThis.fetch;
    const prevFileReader = globalThis.FileReader;
    globalThis.window = {
        getComputedStyle(node) {
            computedCalls++;
            return { cssText: node._cssText, backgroundImage: node._bg, length: 0 };
        },
    };
    const fetchedUrls = [];
    globalThis.fetch = async (url) => {
        fetchedUrls.push(url);
        if (url.includes("broken")) return { ok: false };
        return { ok: true, blob: async () => url };
    };
    globalThis.FileReader = class {
        readAsDataURL(blob) {
            this.result = "data:image/png;base64,INLINE_" + blob.split("/").pop();
            this.onload();
        }
    };
    try {
        const bgTargets = globalThis.__inlineStyles(srcRoot, clRoot);
        await globalThis.__inlineMedia(srcRoot, clRoot, bgTargets);
        assert.strictEqual(computedCalls, 4, "getComputedStyle runs exactly once per node (single pass)");
        assert.strictEqual(
            fetchedUrls.filter((u) => u === "https://shop.test/logo.png").length,
            1,
            "duplicate <img> URL is fetched only once per capture",
        );
        assert.strictEqual(clImg1.getAttribute("src"), "data:image/png;base64,INLINE_logo.png");
        assert.strictEqual(clImg2.getAttribute("src"), "data:image/png;base64,INLINE_logo.png");
        assert.strictEqual(
            clRoot.style.backgroundImage,
            'url("data:image/png;base64,INLINE_bg.png")',
            "root receipt element background-image is inlined",
        );
        assert.strictEqual(
            clDiv.style.backgroundImage,
            "none",
            "unfetchable background-image falls back to none",
        );
    } finally {
        globalThis.window = prevWindow;
        globalThis.fetch = prevFetch;
        globalThis.FileReader = prevFileReader;
    }
}


// Regression tests for POS_CAPTURE_FAILED root causes:
// 1) inlineStyles strips external url(...) from cssText while preserving data: URLs.
// 2) inlineMedia strips srcset from cloned <img> elements.
// 3) elementToPngBytes uses data:image/svg+xml (never URL.createObjectURL which taints WebView2 canvas).
// 4) renderReceiptCanvasFallback produces PNG bytes via 2D canvas without SVG.
{
    const helpers = [
        "const MEDIA_FETCH_TIMEOUT_MS = 8000;",
        'const TRANSPARENT_PIXEL = "data:image/png;base64,TRANSPARENT";',
        (src.match(/const RECEIPT_LAYOUT_FIX_CSS = [\s\S]*?;/)?.[0] || 'const RECEIPT_LAYOUT_FIX_CSS = "";'),
        extract("inlineStyles"),
        extract("fetchAsDataUrl"),
        extract("inlineMedia"),
        extract("canvasToPngBytes"),
        extract("withConnectedElement"),
        extract("elementToPngBytes"),
        extract("renderReceiptCanvasFallback"),
        "globalThis.__testCapture = { inlineStyles, inlineMedia, elementToPngBytes, renderReceiptCanvasFallback };",
    ].join("\n");
    eval(helpers);

    const removedAttrs = [];
    const makeEl = (tag, { attrs = {}, bg = "none", cssText = "", children = [] } = {}) => {
        const el = {
            tagName: tag.toUpperCase(),
            isConnected: true,
            offsetWidth: 300,
            offsetHeight: 400,
            scrollHeight: 400,
            _attrs: { ...attrs },
            _bg: bg,
            _cssText: cssText,
            style: {},
            children,
            get src() { return this._attrs.src || ""; },
            get currentSrc() { return this._attrs.src || ""; },
            getAttribute(k) { return this._attrs[k] ?? null; },
            setAttribute(k, v) { this._attrs[k] = String(v); },
            removeAttribute(k) { removedAttrs.push(k); delete this._attrs[k]; },
            getBoundingClientRect() { return { left: 0, top: 0, right: 300, bottom: 400, width: 300, height: 400 }; },
            querySelectorAll(sel) {
                const tags = sel.split(",").map((s) => s.trim().toUpperCase());
                const out = [];
                const walk = (n) => {
                    for (const c of n.children) {
                        if (tags.includes("*") || tags.includes(c.tagName)) out.push(c);
                        walk(c);
                    }
                };
                walk(this);
                return out;
            },
            cloneNode() {
                return makeEl(tag, {
                    attrs: { ...this._attrs },
                    bg: this._bg,
                    cssText: this._cssText,
                    children: this.children.map((c) => c.cloneNode(true)),
                });
            },
        };
        return el;
    };

    const srcImg = makeEl("img", { attrs: { src: "https://shop.test/logo.png", srcset: "https://shop.test/logo@2x.png 2x" } });
    const srcRoot = makeEl("div", {
        attrs: { dir: "rtl" },
        cssText: 'color:black;mask-image:url("https://shop.test/mask.svg");background-image:url("data:image/png;base64,AA==");',
        children: [srcImg],
    });

    const prevWindow = globalThis.window;
    const prevDocument = globalThis.document;
    const prevFetch = globalThis.fetch;
    const prevFileReader = globalThis.FileReader;
    const prevImage = globalThis.Image;
    const prevXMLSerializer = globalThis.XMLSerializer;
    const prevURL = globalThis.URL;

    let createObjectUrlCalled = false;
    let loadedImgSrc = "";
    globalThis.window = {
        getComputedStyle(node) {
            return { cssText: node._cssText, backgroundImage: node._bg, direction: "rtl", length: 0 };
        },
    };
    globalThis.document = {
        documentElement: { dir: "rtl", lang: "ar" },
        createElement(tag) {
            if (tag === "canvas") {
                return {
                    width: 0,
                    height: 0,
                    getContext() {
                        return {
                            fillStyle: "",
                            fillRect() {},
                            scale() {},
                            drawImage() {},
                            fillText() {},
                        };
                    },
                    toBlob(cb) {
                        cb({ arrayBuffer: async () => new Uint8Array([137, 80, 78, 71]).buffer });
                    },
                };
            }
            return makeEl(tag);
        },
    };
    globalThis.fetch = async () => ({ ok: false });
    globalThis.FileReader = class { readAsDataURL() { this.onload(); } };
    globalThis.XMLSerializer = class { serializeToString(node) { return `<div style="${node.getAttribute("style")}" dir="${node.getAttribute("dir")}"></div>`; } };
    globalThis.Image = class {
        set src(v) {
            loadedImgSrc = v;
            Promise.resolve().then(() => this.onload && this.onload());
        }
    };
    globalThis.URL = {
        createObjectURL() { createObjectUrlCalled = true; return "blob:bad"; },
        revokeObjectURL() {},
    };
    try {
        const bytes = await globalThis.__testCapture.elementToPngBytes(srcRoot, 2);
        assert.deepStrictEqual(bytes, [137, 80, 78, 71], "SVG capture produces PNG bytes");
        assert.strictEqual(createObjectUrlCalled, false, "must never use URL.createObjectURL for foreignObject SVG");
        assert.ok(loadedImgSrc.startsWith("data:image/svg+xml;charset=utf-8,"), "uses data:image/svg+xml URI");
        const decodedSvg = decodeURIComponent(loadedImgSrc);
        const expW = srcRoot.offsetWidth || 320;
        const expH = srcRoot.offsetHeight || srcRoot.scrollHeight || 480;
        assert.ok(decodedSvg.includes(`width="${expW * 2}" height="${expH * 2}" viewBox="0 0 ${expW} ${expH}"`), "SVG rasterizes natively at scaled dimensions via viewBox");
        assert.ok(!decodeURIComponent(loadedImgSrc).includes("mask.svg"), "external CSS url(...) stripped");
        assert.ok(decodeURIComponent(loadedImgSrc).includes("data:image/png;base64,AA=="), "data: CSS url preserved");
        assert.ok(removedAttrs.includes("srcset"), "srcset stripped from cloned img");

        const fallbackBytes = await globalThis.__testCapture.renderReceiptCanvasFallback(srcRoot, 2);
        assert.deepStrictEqual(fallbackBytes, [137, 80, 78, 71], "2D canvas fallback produces PNG bytes");

        // Detached element with an existing offscreen parent is temporarily mounted and restored.
        let restoredToOrigParent = false;
        let hostRemoved = false;
        const origParent = {
            insertBefore(child) {
                if (child === detachedEl) restoredToOrigParent = true;
            },
        };
        const detachedEl = makeEl("div", { cssText: "color:black;" });
        detachedEl.isConnected = false;
        detachedEl.parentNode = origParent;
        detachedEl.nextSibling = null;
        globalThis.document.body = { appendChild() {} };
        const origCreateEl = globalThis.document.createElement;
        globalThis.document.createElement = (tag) => {
            const el = origCreateEl(tag);
            if (tag === "div") {
                el.appendChild = () => {};
                el.remove = () => { hostRemoved = true; };
            }
            return el;
        };
        const detachedBytes = await globalThis.__testCapture.elementToPngBytes(detachedEl, 1);
        assert.deepStrictEqual(detachedBytes, [137, 80, 78, 71], "detached element captures PNG bytes");
        assert.strictEqual(restoredToOrigParent, true, "detached element restored to original parent after capture");
        assert.strictEqual(hostRemoved, true, "temporary offscreen host removed after capture");
        globalThis.document.createElement = origCreateEl;
        delete globalThis.document.body;

        // Hung SVG Image load times out instead of locking inFlightPosKeys forever.
        const origImage2 = globalThis.Image;
        const origSetTimeoutPos = globalThis.setTimeout;
        globalThis.Image = class { set src(_v) { /* never fires onload/onerror */ } };
        globalThis.setTimeout = (fn, ms) => {
            if (ms === 8000) {
                queueMicrotask(fn);
                return 9001;
            }
            return origSetTimeoutPos(fn, ms);
        };
        try {
            let timedOut = false;
            try {
                await globalThis.__testCapture.elementToPngBytes(srcRoot, 1);
            } catch (err) {
                timedOut = /timed out/i.test(err.message);
            }
            assert.strictEqual(timedOut, true, "hung SVG Image load rejects with timeout");
        } finally {
            globalThis.Image = origImage2;
            globalThis.setTimeout = origSetTimeoutPos;
        }
    } finally {
        globalThis.window = prevWindow;
        globalThis.document = prevDocument;
        globalThis.fetch = prevFetch;
        globalThis.FileReader = prevFileReader;
        globalThis.Image = prevImage;
        globalThis.XMLSerializer = prevXMLSerializer;
        globalThis.URL = prevURL;
    }
}

// Event-driven systray refresh from POS watcher and declined attempts.
{
    const rpcCalls = [];
    let jobUpdatedEvents = 0;
    const prevWindow = globalThis.window;
    const prevSetTimeout = globalThis.setTimeout;
    globalThis.window = {
        dispatchEvent(ev) {
            if (ev && ev.type === "native_print:job_updated") jobUpdatedEvents++;
            return true;
        },
    };
    globalThis.setTimeout = (fn) => { fn(); return 0; };
    try {
        const posWatchHelpers = [
            'const POS_RECEIPT_SOURCE = "point_of_sale.receipt";',
            'const TERMINAL_STATES = new Set(["done", "failed", "canceled", "dead_letter"]);',
            'const _t = (s, ...args) => args.reduce((acc, v) => acc.replace("%s", String(v)), s);',
            'const notifyUser = (_env, msg, type) => { globalThis.__posNotices.push({ msg, type }); };',
            'const safeErrorCode = (code, fallback = "error") => { const t = typeof code === "string" ? code.trim() : ""; return /^[A-Za-z0-9_.-]{1,64}$/.test(t) ? t : fallback; };',
            'const notifyJobUpdated = () => { if (typeof window !== "undefined" && typeof window.dispatchEvent === "function") window.dispatchEvent(typeof CustomEvent === "function" ? new CustomEvent("native_print:job_updated") : new Event("native_print:job_updated")); };',
            'const rpc = async (url, params) => { globalThis.__posRpcCalls.push({ url, params }); return { ok: true }; };',
            extract("finishPosReceiptAttempt"),
            extract("watchPosJob"),
            extract("postPosDeclined"),
            "globalThis.__watchPosJob = watchPosJob; globalThis.__postPosDeclined = postPosDeclined;",
        ].join("\n");
        const posNotices = [];
        globalThis.__posRpcCalls = rpcCalls;
        globalThis.__posNotices = posNotices;
        eval(posWatchHelpers);

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
        assert.strictEqual(posNotices.length, 1, "watchPosJob notifies user on done");
        assert.strictEqual(posNotices[0].type, "success", "done toast uses success severity");
        posNotices.length = 0;

        globalThis.__postPosDeclined("pos-decl-1", "POS_NO_PRINTER", "Order_Ev2", "7", "EPSON", "escpos");
        await new Promise((r) => prevSetTimeout(r, 10));
        assert.strictEqual(jobUpdatedEvents, 2, "postPosDeclined dispatches native_print:job_updated");

        // Failure path: dead_letter sanitizes multi-line trace errorCode via safeErrorCode without ReferenceError.
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
        assert.strictEqual(posNotices[0].type, "danger", "dead_letter toast uses danger severity");
        assert.ok(posNotices[0].msg.includes("(dead_letter)"), "multi-line errorCode falls back to state");
        assert.ok(!posNotices[0].msg.includes("SECRET_TRACE"), "raw trace is never leaked into POS toast");
    } finally {
        globalThis.window = prevWindow;
        globalThis.setTimeout = prevSetTimeout;
        delete globalThis.__posRpcCalls;
        delete globalThis.__posNotices;
    }
}

// Android v1.5.7 POS watcher: polls by bridgeJobId and normalizes `completed` -> `done`.
{
    const rpcCalls = [];
    const polled = [];
    const prevWindow = globalThis.window;
    const prevSetTimeout = globalThis.setTimeout;
    globalThis.window = { dispatchEvent: () => true };
    globalThis.setTimeout = (fn) => { fn(); return 0; };
    try {
        const helpers = [
            'const POS_RECEIPT_SOURCE = "point_of_sale.receipt";',
            'const TERMINAL_STATES = new Set(["done", "failed", "canceled", "dead_letter"]);',
            'const normalizeBridgeJobState = (s) => ({ completed: "done", cancelled: "canceled", submitted: "printing", in_progress: "printing" }[String(s || "").toLowerCase()] || s);',
            'const _t = (s, ...args) => args.reduce((acc, v) => acc.replace("%s", String(v)), s);',
            'const notifyUser = () => {};',
            'const safeErrorCode = (c, f = "error") => c || f;',
            'const notifyJobUpdated = () => {};',
            'const rpc = async (url, params) => { globalThis.__andPosRpcs.push({ url, params }); return { ok: true }; };',
            extract("finishPosReceiptAttempt"),
            extract("watchPosJob"),
            "globalThis.__andWatchPos = watchPosJob;",
        ].join("\n");
        globalThis.__andPosRpcs = rpcCalls;
        eval(helpers);

        const order = { _printed: false };
        await globalThis.__andWatchPos(
            { jobDetail: async (id) => { polled.push(id); return { id, state: "completed", attempts: 1 }; } },
            "pos-Order_And-20261125-u7-a1",
            { printer_pattern: "BT Printer", copies: 1, mode: "escpos" },
            64,
            "Order_And",
            "BT Printer",
            null,
            "7",
            order,
            "android-pos-uuid-1",
        );
        assert.strictEqual(polled[0], "android-pos-uuid-1", "watchPosJob polls Android bridgeJobId");
        assert.strictEqual(order._printed, true, "Android completed marks order._printed = true");
        assert.strictEqual(rpcCalls.length, 1, "watchPosJob posts one terminal status");
        assert.strictEqual(rpcCalls[0].params.idempotency_key, "pos-Order_And-20261125-u7-a1", "RPC keeps idempotency_key");
        assert.strictEqual(rpcCalls[0].params.state, "done", "Android completed synced as done");
    } finally {
        globalThis.window = prevWindow;
        globalThis.setTimeout = prevSetTimeout;
        delete globalThis.__andPosRpcs;
    }
}


// 11. watchPosJob surfaces friendly license message for ERR_LICENSE_REQUIRED / ERR_LICENSE_EXPIRED.
{
    const rpcCalls = [];
    const posNotices = [];
    const prevWindow = globalThis.window;
    const prevSetTimeout = globalThis.setTimeout;
    globalThis.window = { dispatchEvent: () => true };
    globalThis.setTimeout = (fn) => { fn(); return 0; };
    try {
        const helpers = [
            'const POS_RECEIPT_SOURCE = "point_of_sale.receipt";',
            'const TERMINAL_STATES = new Set(["done", "failed", "canceled", "dead_letter"]);',
            'const normalizeBridgeJobState = (s) => s;',
            'const _t = (s, ...args) => args.reduce((acc, v) => acc.replace("%s", String(v)), s);',
            'const notifyUser = (_env, msg, type) => globalThis.__licPosNotices.push({ msg, type });',
            'const safeErrorCode = (c, f = "error") => c || f;',
            'const licenseErrorMessage = (c) => c === "ERR_LICENSE_EXPIRED" ? "Your Itbaa license has expired." : c === "ERR_LICENSE_REQUIRED" ? "An Itbaa license key is required." : null;',
            'const notifyJobUpdated = () => {};',
            'const rpc = async (url, params) => { globalThis.__licPosRpcs.push({ url, params }); return { ok: true }; };',
            extract("finishPosReceiptAttempt"),
            extract("watchPosJob"),
            "globalThis.__licWatchPos = watchPosJob;",
        ].join("\n");
        globalThis.__licPosRpcs = rpcCalls;
        globalThis.__licPosNotices = posNotices;
        eval(helpers);

        const order = { _printed: true };
        await globalThis.__licWatchPos(
            { jobDetail: async () => ({ state: "dead_letter", errorCode: "ERR_LICENSE_EXPIRED", attempts: 1 }) },
            "pos-lic-1",
            { printer_pattern: "EPSON", copies: 1, mode: "escpos" },
            64,
            "Order_Lic",
            "EPSON",
            null,
            "7",
            order,
        );
        assert.strictEqual(order._printed, false, "watchPosJob clears _printed on ERR_LICENSE_EXPIRED");
        assert.strictEqual(rpcCalls[0].params.error_code, "ERR_LICENSE_EXPIRED", "watchPosJob logs ERR_LICENSE_EXPIRED");
        assert.strictEqual(posNotices.length, 1);
        assert.ok(/license has expired/i.test(posNotices[0].msg), "watchPosJob surfaces friendly expired-license message");
    } finally {
        globalThis.window = prevWindow;
        globalThis.setTimeout = prevSetTimeout;
        delete globalThis.__licPosRpcs;
        delete globalThis.__licPosNotices;
    }
}

console.log("pos key tests: ALL PASS");

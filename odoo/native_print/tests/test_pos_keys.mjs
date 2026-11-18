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
const store = new Map();
globalThis.sessionStorage = {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
};
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
// Audit H1: the SAME order printed by a DIFFERENT cashier is a
// different job — different key, and its own in-flight record (the
// first cashier's record must not dedupe or clear it).
const other = globalThis.__k("Order_1", "9");
assert.match(other, /^pos-Order_1-\d{8}-u9-a1$/);
assert.notStrictEqual(other, k3);
assert.strictEqual(globalThis.__k("Order_1", "9"), other, "second cashier keeps own in-flight key");
assert.strictEqual(globalThis.__k("Order_1", "7"), k3, "first cashier record untouched");
// Stale record (>15s without terminal state) advances to a fresh attempt.
store.set("np.pos.key.Order_3.u7", JSON.stringify({ key: "pos-Order_3-20261119-u7-a1", ts: Date.now() - 20000, attempt: 1 }));
const staleNext = globalThis.__k("Order_3", "7");
assert.match(staleNext, /-a2$/, "stale >15s record advances to next attempt");

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

// Phase 3 regression test: single-pass getComputedStyle, root backgroundImage inlining,
// URL fetch deduplication, and timeout/error fallback in inlineStyles + inlineMedia.
{
    const helpers = [
        "const MEDIA_FETCH_TIMEOUT_MS = 8000;",
        'const TRANSPARENT_PIXEL = "data:image/png;base64,TRANSPARENT";',
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

console.log("pos key tests: ALL PASS");

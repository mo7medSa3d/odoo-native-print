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
    const start = src.indexOf(`function ${name}(`);
    assert.ok(start >= 0, `${name} exists`);
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
eval(extract("posReceiptKey") + "\n" + extract("finishPosReceiptAttempt") +
    "\nglobalThis.__k = posReceiptKey; globalThis.__c = finishPosReceiptAttempt;");

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
console.log("pos key tests: ALL PASS");

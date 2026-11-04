/** @odoo-module **/
import { ReceiptScreen } from "@point_of_sale/app/screens/receipt_screen/receipt_screen";
import { patch } from "@web/core/utils/patch";
import { rpc } from "@web/core/network/rpc";
import { _t } from "@web/core/l10n/translation";
import { notifyUser, resolveBridgeTarget } from "./native_print_client";

/*
 * Native Print for POS receipts (Odoo 17, 18, and 19 compatible).
 *
 * Architectural compatibility:
 * - In Odoo 17, `printReceipt` is a method on `ReceiptScreen.prototype`.
 * - In Odoo 18 and 19, `printReceipt` moved to `PosStore.prototype`
 *   (`this.pos.printReceipt`) and `ReceiptScreen.setup()` invokes it
 *   via `this.doFullPrint = useTrackedAsync(() => this.pos.printReceipt())`.
 *
 * We hook BOTH:
 * 1. In `ReceiptScreen.prototype.setup()`, we wrap `this.pos.printReceipt`
 *    on the PosStore instance/prototype (idempotently) so manual prints,
 *    auto-prints, and TicketScreen reprints all route through Native Print.
 * 2. When no `.pos-receipt` DOM exists in the document (e.g. a
 *    TicketScreen reprint before ReceiptScreen mounts), the native
 *    path declines (returns false) and Odoo's standard POS printing
 *    runs unchanged — there is no offscreen rendering path.
 * 3. We also patch `ReceiptScreen.prototype.printReceipt` for Odoo 17.
 */

const POS_RECEIPT_SOURCE = "point_of_sale.receipt";
const TERMINAL_STATES = new Set(["done", "failed", "canceled", "dead_letter"]);
// The virtual-printer warning is shown once per POS session, not
// once per receipt.
let virtualPrinterNoticeShown = false;

function nativeBridge() {
    return window.__NATIVE_PRINT__ || null;
}

function notificationEnv(contextObj) {
    // OWL components expose env directly, and the PosStore keeps it
    // on .env as well. Anything else simply means no toast is
    // possible here; notifyUser then no-ops.
    return (
        (contextObj && contextObj.env) ||
        (contextObj && contextObj.pos && contextObj.pos.env) ||
        null
    );
}

function notifyPosNativeFailure(contextObj) {
    // One clear message when native receipt printing gives up
    // inside the desktop app. Nothing was printed: the receipt
    // stays on screen and the cashier can fix the cause and click
    // Print again. Identical repeats are deduped inside notifyUser,
    // so the two wrapper layers below can never stack this toast.
    notifyUser(
        notificationEnv(contextObj),
        _t("Native Print could not print this receipt, and nothing was printed. Check the printer and try again."),
        "warning",
    );
}

function findReceiptElement(orderRef) {
    if (orderRef) {
        // Prefer the receipt that actually belongs to this order:
        // after fast order switching the DOM can still hold the
        // previous order's receipt, and printing it would put the
        // wrong receipt on paper (audit L8).
        const candidates = document.querySelectorAll(".pos-receipt");
        for (const el of candidates) {
            if (el.textContent && el.textContent.includes(orderRef)) {
                return el;
            }
        }
        if (candidates.length) {
            // A receipt is on screen but it is not this order's:
            // decline and let the standard POS printing handle it.
            return null;
        }
    }
    return (
        document.querySelector(".pos-receipt-container .pos-receipt") ||
        document.querySelector(".pos-receipt") ||
        document.querySelector(".pos-receipt-container")
    );
}

function inlineStyles(source, clone) {
    const computed = window.getComputedStyle(source);
    let css = "";
    for (let i = 0; i < computed.length; i++) {
        const prop = computed[i];
        css += `${prop}:${computed.getPropertyValue(prop)};`;
    }
    clone.setAttribute("style", css);
    const sourceKids = source.children;
    const cloneKids = clone.children;
    for (let i = 0; i < sourceKids.length && i < cloneKids.length; i++) {
        inlineStyles(sourceKids[i], cloneKids[i]);
    }
}

async function fetchAsDataUrl(url) {
    try {
        const resp = await fetch(url, { credentials: "same-origin" });
        if (!resp.ok) {
            return null;
        }
        const blob = await resp.blob();
        return await new Promise((resolve) => {
            const reader = new FileReader();
            reader.onload = () => resolve(reader.result);
            reader.onerror = () => resolve(null);
            reader.readAsDataURL(blob);
        });
    } catch (_err) {
        return null;
    }
}

async function inlineMedia(source, clone) {
    const sourceImgs = source.querySelectorAll("img");
    const cloneImgs = clone.querySelectorAll("img");
    for (let i = 0; i < sourceImgs.length && i < cloneImgs.length; i++) {
        const src = sourceImgs[i].currentSrc || sourceImgs[i].src;
        if (src && !src.startsWith("data:")) {
            const dataUrl = await fetchAsDataUrl(src);
            if (dataUrl) {
                cloneImgs[i].setAttribute("src", dataUrl);
            }
        }
    }
    const sourceCanvases = source.querySelectorAll("canvas");
    const cloneCanvases = clone.querySelectorAll("canvas");
    for (let i = 0; i < sourceCanvases.length && i < cloneCanvases.length; i++) {
        try {
            const img = document.createElement("img");
            img.setAttribute("src", sourceCanvases[i].toDataURL("image/png"));
            img.setAttribute("width", sourceCanvases[i].width);
            img.setAttribute("height", sourceCanvases[i].height);
            cloneCanvases[i].replaceWith(img);
        } catch (_err) {
            // Leave cross-origin canvas as-is.
        }
    }
    // CSS background images (shop logos are often background art,
    // not <img>): the SVG rasterizer cannot load external URLs, so
    // an un-inlined background silently vanishes from the printed
    // receipt (audit JS-L9). Fetch them to data URLs like <img>s.
    const sourceAll = source.querySelectorAll("*");
    const cloneAll = clone.querySelectorAll("*");
    for (let i = 0; i < sourceAll.length && i < cloneAll.length; i++) {
        const bg = window.getComputedStyle(sourceAll[i]).backgroundImage;
        const match = bg && bg.match(/url\(["']?(.*?)["']?\)/);
        if (match && match[1] && !match[1].startsWith("data:")) {
            const dataUrl = await fetchAsDataUrl(match[1]);
            if (dataUrl) {
                cloneAll[i].style.backgroundImage = `url("${dataUrl}")`;
            }
        }
    }
}

async function elementToPngBytes(element, scale = 2) {
    const width = element.offsetWidth || 320;
    const height = element.offsetHeight || element.scrollHeight || 480;
    if (!width || !height) {
        throw new Error("receipt element has no size");
    }
    const clone = element.cloneNode(true);
    inlineStyles(element, clone);
    await inlineMedia(element, clone);
    clone.setAttribute("xmlns", "http://www.w3.org/1999/xhtml");
    const svg =
        `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">` +
        `<foreignObject width="100%" height="100%">` +
        new XMLSerializer().serializeToString(clone) +
        `</foreignObject></svg>`;
    const img = new Image();
    const loaded = new Promise((resolve, reject) => {
        img.onload = resolve;
        img.onerror = () => reject(new Error("receipt SVG failed to load"));
    });
    img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg);
    await loaded;
    const canvas = document.createElement("canvas");
    canvas.width = Math.ceil(width * scale);
    canvas.height = Math.ceil(height * scale);
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.scale(scale, scale);
    ctx.drawImage(img, 0, 0, width, height);
    const blob = await new Promise((resolve) =>
        canvas.toBlob(resolve, "image/png"),
    );
    if (!blob) {
        throw new Error("receipt canvas produced no image");
    }
    return Array.from(new Uint8Array(await blob.arrayBuffer()));
}

function clearPosReceiptKey(safeRef, userId) {
    try { sessionStorage.removeItem(`np.pos.key.${safeRef}.u${userId}`); } catch (_err) { /* ignore */ }
}

function posReceiptKey(safeRef, userId) {
    // Idempotency with intent: resubmitting the SAME order within a
    // few seconds is the same print (double-fire, POS retry) and must
    // dedupe on the desktop, so it reuses the in-flight key. Once the
    // job reaches a terminal state the watcher clears the record, so
    // a later deliberate reprint becomes a fresh attempt (a2, a3...).
    // The day stamp bounds key reuse to one business day.
    // The cashier's user id is part of BOTH the storage key and the
    // job key (audit H1): the server log scopes jobs per user, so a
    // deterministic key shared across cashiers made the second
    // cashier's identical order a forbidden collision — her print
    // vanished from the log and her watcher read the first cashier's
    // job. Per-user keys keep the dedupe semantics per cashier.
    const storeKey = `np.pos.key.${safeRef}.u${userId}`;
    const now = Date.now();
    let record = null;
    try {
        record = JSON.parse(sessionStorage.getItem(storeKey) || "null");
    } catch (_err) { /* storage unavailable: fall through */ }
    if (record && record.key && now - (record.ts || 0) < 15000) {
        // The 15 s window stays anchored at the first submission:
        // refreshing it on every reuse would let repeated prints
        // dedupe into one desktop job indefinitely. (Audit F12.)
        try {
            sessionStorage.setItem(storeKey, JSON.stringify(record));
        } catch (_err) { /* ignore */ }
        return record.key;
    }
    const attempt = (record && record.attempt ? record.attempt : 0) + 1;
    const day = new Date().toISOString().slice(0, 10).replace(/-/g, "");
    const key = `pos-${safeRef}-${day}-u${userId}-a${attempt}`;
    try {
        sessionStorage.setItem(
            storeKey, JSON.stringify({ key, ts: now, attempt }));
    } catch (_err) { /* ignore */ }
    return key;
}

function finishPosReceiptAttempt(safeRef, userId) {
    // Terminal state seen: the NEXT print of this order is a new
    // attempt. Keep the attempt counter as a tombstone (key cleared)
    // — deleting the record would reset the counter and reissue the
    // very same key, which the desktop would dedupe into silence.
    try {
        const storeKey = `np.pos.key.${safeRef}.u${userId}`;
        const record = JSON.parse(sessionStorage.getItem(storeKey) || "null");
        sessionStorage.setItem(storeKey, JSON.stringify({
            key: null, ts: 0, attempt: record && record.attempt ? record.attempt : 1,
        }));
    } catch (_err) { /* ignore */ }
}

async function watchPosJob(bridge, jobId, rule, byteSize, safeRef, printerName, env, userId, order) {
    // Same watch policy as report jobs: poll until terminal, giving
    // up only after 5 minutes without any state progress (a busy
    // queue legitimately takes longer), 30-minute absolute cap.
    const absoluteDeadline = Date.now() + 30 * 60 * 1000;
    let lastProgressAt = Date.now();
    let lastSeenState = null;
    let reachedTerminal = false;
    let consecutiveErrors = 0;
    for (let i = 0; Date.now() < absoluteDeadline && Date.now() - lastProgressAt < 5 * 60 * 1000; i++) {
        await new Promise((resolve) =>
            setTimeout(resolve, i < 10 ? 600 : 2000));
        try {
            let state = null;
            let errorCode = null;
            let attempts = 1;
            if (typeof bridge.jobDetail === "function") {
                const detail = await bridge.jobDetail(jobId);
                if (detail) {
                    state = detail.state;
                    errorCode = detail.errorCode || null;
                    attempts = detail.attempts || attempts;
                }
            } else if (typeof bridge.printStatus === "function") {
                state = await bridge.printStatus(jobId);
            }
            consecutiveErrors = 0;
            if (state && state !== lastSeenState) {
                lastSeenState = state;
                lastProgressAt = Date.now();
            }
            if (state && TERMINAL_STATES.has(state)) {
                reachedTerminal = true;
                // Terminal: the next print of this order is a new,
                // deliberate attempt with a fresh idempotency key.
                if (safeRef) {
                    finishPosReceiptAttempt(safeRef, userId);
                }
                // The order's printed flag follows the CONFIRMED
                // outcome, not the enqueue: it is pre-set at enqueue
                // so the POS flow can advance, and a failed job must
                // hand the order back as unprinted so the cashier's
                // normal reprint path stays available (audit L7).
                if (order) {
                    if (state === "done") {
                        order._printed = true;
                    } else if (state === "failed" || state === "dead_letter") {
                        order._printed = false;
                    }
                }
                await rpc(
                    "/native_print/job_status",
                    {
                        idempotency_key: jobId,
                        state,
                        error_code: errorCode,
                        device: printerName || rule.printer_pattern,
                        report_name: POS_RECEIPT_SOURCE,
                        source_type: "pos_receipt",
                        pages: 1,
                        byte_size: byteSize,
                        attempts,
                    },
                    { silent: true },
                ).catch(() => {});
                if (state === "failed" || state === "dead_letter") {
                    // The cashier must know this receipt failed;
                    // success stays silent (the receipt itself, plus
                    // the receipt screen, is the feedback). Wording
                    // stays honest: a failed submission can still
                    // have produced partial output on some paths, so
                    // the cashier is told to check before reprinting.
                    notifyUser(
                        env,
                        _t('Receipt printing failed on "%s" (%s). Check the printer before reprinting.', printerName || rule.printer_pattern, errorCode || state),
                        "danger",
                    );
                }
                break;
            }
        } catch (_err) {
            // Same rule as report jobs: one flaky poll must not
            // orphan the job and hide its outcome from the cashier.
            if (++consecutiveErrors >= 3) {
                break;
            }
        }
    }
    if (!reachedTerminal) {
        notifyUser(
            env,
            _t('Receipt job on "%s" is taking long; its final outcome is unconfirmed. Check Print Jobs later.', printerName || rule.printer_pattern),
            "warning",
        );
    }

}

async function printPosReceiptNatively(contextObj, explicitOrder = null) {
    const bridge = nativeBridge();
    if (!bridge || typeof bridge.printImage !== "function") {
        return false;
    }
    const target = await resolveBridgeTarget(
        bridge, { preferClasses: ["thermal", "label"] },
    ).catch(() => null);
    if (!target) {
        return false; // No printer on this PC right now.
    }
    // A virtual printer (Print to PDF, OneNote, ...) cannot print
    // silently: its driver asks the user where to save the output.
    // Say so once per session, up front, instead of letting the job
    // fail later with a cryptic spooler error. The attempt itself
    // still runs — a virtual printer configured with a fixed output
    // path can accept it without asking.
    if (target.printer && target.printer.kind === "virtual" && !virtualPrinterNoticeShown) {
        virtualPrinterNoticeShown = true;
        notifyUser(
            notificationEnv(contextObj),
            _t('The current printer "%s" is a virtual printer: it asks where to save instead of printing silently. For silent receipt printing, pick a physical printer in the Yaseir Print panel.', target.printer.name),
            "warning",
        );
    }
    const order =
        explicitOrder ||
        (contextObj && contextObj.currentOrder) ||
        (contextObj && contextObj.getOrder ? contextObj.getOrder() : null) ||
        (contextObj && contextObj.pos && contextObj.pos.getOrder
            ? contextObj.pos.getOrder()
            : null);
    const rawRef =
        (order && (order.name || order.pos_reference || order.uid)) ||
        "pos-order";
    const safeRef = String(rawRef).replace(/[^A-Za-z0-9_-]/g, "_").slice(0, 40);
    // The cashier printing right now (audit H1): part of the job key
    // and of the in-flight record, because the server log scopes jobs
    // per user and one browser profile can serve several cashiers.
    const posStore = (contextObj && contextObj.pos) || contextObj || null;
    const cashierRaw =
        (posStore && posStore.user && (posStore.user.id || posStore.user)) ||
        (posStore && posStore.session && posStore.session.user_id) ||
        (order && order.user_id && (order.user_id.id || order.user_id)) ||
        0;
    const userId = String(cashierRaw).replace(/[^0-9]/g, "") || "0";
    const element = findReceiptElement(order ? rawRef : null);
    if (!element) {
        return false;
    }
    const bytes = await elementToPngBytes(element, 2);
    const idempotencyKey = posReceiptKey(safeRef, userId);
    const rule = target.rule;
    try {
        await bridge.printImage({
            idempotencyKey,
            reportName: POS_RECEIPT_SOURCE,
            printerPattern: rule.printer_pattern,
            mode: rule.mode || "escpos",
            // The driver-reported roll width is resolved by the native worker.
            // Never guess 58mm or 80mm for a kitchen/POS receipt.
            mediaRollWidthMm:
                (rule.media && rule.media.roll_width_mm) || undefined,
            copies: rule.copies || 1,
            bytes,
        });
    } catch (err) {
        // The enqueue itself failed: rotate the idempotency record so
        // an immediate deliberate reprint is a fresh attempt instead
        // of deduping into the dead job inside the 15 s window.
        clearPosReceiptKey(safeRef, userId);
        throw err;
    }
    rpc(
        "/native_print/job_status",
        {
            idempotency_key: idempotencyKey,
            state: "queued",
            device: target.printer.name,
            report_name: POS_RECEIPT_SOURCE,
            source_type: "pos_receipt",
            copies: rule.copies || 1,
            mode: rule.mode || undefined,
            pages: 1,
            byte_size: bytes.length,
        },
        { silent: true },
    ).catch(() => {});
    watchPosJob(
        bridge, idempotencyKey, rule, bytes.length, safeRef,
        target.printer.name, notificationEnv(contextObj), userId, order,
    ).catch(() => {});
    if (order) {
        // Provisional: lets the POS flow advance to the receipt
        // screen. The watcher corrects it to the confirmed outcome
        // (clears it again when the job fails). (Audit L7.)
        order._printed = true;
    }
    return true;
}

function wrapPosStorePrintReceipt(posStore) {
    if (!posStore || posStore.__nativePrintWrapped || typeof posStore.printReceipt !== "function") {
        return;
    }
    const originalPrintReceipt = posStore.printReceipt.bind(posStore);
    posStore.printReceipt = async function (opts = {}) {
        if (!nativeBridge()) {
            // Plain browser: Odoo's standard printing is the only
            // printing there is (owner's standing rule for browsers).
            return await originalPrintReceipt(opts);
        }
        // Inside the desktop app a receipt prints silently through
        // the bridge — or not at all. Odoo's standard flow ends in
        // the browser print dialog (PosStore.printReceipt passes
        // webPrintFallback: true), which the owner banned from the
        // app after it reappeared in the field (2026-11-04): a
        // declined or failed native attempt reports its reason and
        // leaves the receipt on screen for another click instead of
        // opening that dialog. Basic receipts are captured the same
        // way; the on-screen receipt is rasterized either way, so
        // the basic/full distinction does not change the output.
        try {
            if (await printPosReceiptNatively(posStore, opts && opts.order)) {
                return true;
            }
            notifyPosNativeFailure(posStore);
        } catch (error) {
            console.warn("Native Print: POS receipt capture failed.", error);
            notifyPosNativeFailure(posStore);
        }
        return true;
    };
    posStore.__nativePrintWrapped = true;
}

patch(ReceiptScreen.prototype, {
    setup() {
        super.setup(...arguments);
        if (this.pos) {
            wrapPosStorePrintReceipt(this.pos);
        }
    },

    async printReceipt() {
        const opts = arguments[0];
        if (!nativeBridge()) {
            // Plain browser: keep Odoo's standard flow untouched.
            if (super.printReceipt) {
                return await super.printReceipt(...arguments);
            }
            if (this.pos && typeof this.pos.printReceipt === "function") {
                return await this.pos.printReceipt(...arguments);
            }
            return false;
        }
        // Desktop app: the same rule as the store wrapper — silent
        // native printing or a clear failure message, never the
        // browser print dialog. (Before this fix, a declined or
        // failed attempt continued into the store's original method
        // and that dialog; audit JS-L8's double-capture concern is
        // moot now that the original is never called after an
        // attempt.)
        try {
            if (await printPosReceiptNatively(this, opts && opts.order)) {
                return true;
            }
            notifyPosNativeFailure(this);
        } catch (error) {
            console.warn("Native Print: POS receipt capture failed.", error);
            notifyPosNativeFailure(this);
        }
        return true;
    },
});

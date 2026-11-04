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

function notifyPosFallback(contextObj) {
    // One clear message when native receipt printing gives up and
    // the standard POS printing takes over. Identical repeats are
    // deduped inside notifyUser, so the two wrapper layers below can
    // never stack this toast.
    notifyUser(
        notificationEnv(contextObj),
        _t("Native Print could not print this receipt. The standard POS printing was used instead."),
        "warning",
    );
}

function findReceiptElement() {
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

function posReceiptKey(safeRef) {
    // Idempotency with intent: resubmitting the SAME order within a
    // few seconds is the same print (double-fire, POS retry) and must
    // dedupe on the desktop, so it reuses the in-flight key. Once the
    // job reaches a terminal state the watcher clears the record, so
    // a later deliberate reprint becomes a fresh attempt (a2, a3...).
    // The day stamp bounds key reuse to one business day.
    const storeKey = `np.pos.key.${safeRef}`;
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
    const key = `pos-${safeRef}-${day}-a${attempt}`;
    try {
        sessionStorage.setItem(
            storeKey, JSON.stringify({ key, ts: now, attempt }));
    } catch (_err) { /* ignore */ }
    return key;
}

function finishPosReceiptAttempt(safeRef) {
    // Terminal state seen: the NEXT print of this order is a new
    // attempt. Keep the attempt counter as a tombstone (key cleared)
    // — deleting the record would reset the counter and reissue the
    // very same key, which the desktop would dedupe into silence.
    try {
        const storeKey = `np.pos.key.${safeRef}`;
        const record = JSON.parse(sessionStorage.getItem(storeKey) || "null");
        sessionStorage.setItem(storeKey, JSON.stringify({
            key: null, ts: 0, attempt: record && record.attempt ? record.attempt : 1,
        }));
    } catch (_err) { /* ignore */ }
}

async function watchPosJob(bridge, jobId, rule, byteSize, safeRef, printerName, env) {
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
                    finishPosReceiptAttempt(safeRef);
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
    const element = findReceiptElement();
    if (!element) {
        return false;
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
    const bytes = await elementToPngBytes(element, 2);
    const idempotencyKey = posReceiptKey(safeRef);
    const rule = target.rule;
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
        target.printer.name, notificationEnv(contextObj),
    );
    if (order) {
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
        const isBasic = !!(opts && (opts.basic || opts.basicReceipt));
        if (!isBasic) {
            try {
                if (await printPosReceiptNatively(posStore, opts && opts.order)) {
                    return true;
                }
            } catch (error) {
                console.warn(
                    "Native Print: POS receipt capture failed; falling back to default POS printing.",
                    error,
                );
                notifyPosFallback(posStore);
            }
        }
        return await originalPrintReceipt(opts);
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
        const isBasic = !!(opts && (opts.basic || opts.basicReceipt));
        if (!isBasic) {
            try {
                if (await printPosReceiptNatively(this, opts && opts.order)) {
                    return true;
                }
            } catch (error) {
                console.warn(
                    "Native Print: POS receipt capture failed; falling back to default POS printing.",
                    error,
                );
                notifyPosFallback(this);
            }
        }
        if (super.printReceipt) {
            return await super.printReceipt(...arguments);
        }
        if (this.pos && typeof this.pos.printReceipt === "function") {
            return await this.pos.printReceipt(...arguments);
        }
        return false;
    },
});

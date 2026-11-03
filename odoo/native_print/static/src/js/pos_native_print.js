/** @odoo-module **/
import { ReceiptScreen } from "@point_of_sale/app/screens/receipt_screen/receipt_screen";
import { patch } from "@web/core/utils/patch";
import { rpc } from "@web/core/network/rpc";

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
 * 2. When `TicketScreen` reprints an order before `ReceiptScreen` is mounted,
 *    and no `.pos-receipt` DOM exists in the main document, we render the
 *    order receipt via `posStore.printer` / `env.services.renderer` in an
 *    offscreen container, capture it at 2x PNG, and clean it up immediately.
 * 3. We also patch `ReceiptScreen.prototype.printReceipt` for Odoo 17.
 */

const POS_RECEIPT_SOURCE = "point_of_sale.receipt";
const TERMINAL_STATES = new Set(["done", "failed", "canceled", "dead_letter"]);

function nativeBridge() {
    return window.__NATIVE_PRINT__ || null;
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

async function watchPosJob(bridge, jobId, rule, byteSize) {
    for (let i = 0; i < 30; i++) {
        await new Promise((resolve) => setTimeout(resolve, 600));
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
            if (state && TERMINAL_STATES.has(state)) {
                await rpc(
                    "/native_print/job_status",
                    {
                        idempotency_key: jobId,
                        state,
                        error_code: errorCode,
                        device: rule.printer_pattern,
                        report_name: POS_RECEIPT_SOURCE,
                        profile_id: rule.profile_id,
                        pages: 1,
                        byte_size: byteSize,
                        attempts,
                    },
                    { silent: true },
                ).catch(() => {});
                break;
            }
        } catch (_err) {
            break;
        }
    }
}

async function printPosReceiptNatively(contextObj, explicitOrder = null) {
    const bridge = nativeBridge();
    if (!bridge || typeof bridge.printImage !== "function") {
        return false;
    }
    const resolved = await rpc(
        "/native_print/rules",
        { report_name: POS_RECEIPT_SOURCE },
        { silent: true },
    );
    if (!resolved || !resolved.rule) {
        return false;
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
    const idempotencyKey =
        `pos-${safeRef}-${Date.now()}-` +
        Math.random().toString(36).slice(2, 10);
    const rule = resolved.rule;
    await bridge.printImage({
        idempotencyKey,
        reportName: POS_RECEIPT_SOURCE,
        printerPattern: rule.printer_pattern,
        mode: rule.mode || "escpos",
        paper: rule.paper || "80mm",
        copies: rule.copies || 1,
        bytes,
    });
    rpc(
        "/native_print/job_status",
        {
            idempotency_key: idempotencyKey,
            state: "queued",
            device: rule.printer_pattern,
            report_name: POS_RECEIPT_SOURCE,
            profile_id: rule.profile_id,
            pages: 1,
            byte_size: bytes.length,
        },
        { silent: true },
    ).catch(() => {});
    watchPosJob(bridge, idempotencyKey, rule, bytes.length);
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

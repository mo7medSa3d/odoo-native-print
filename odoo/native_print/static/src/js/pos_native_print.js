/** @odoo-module **/
import { ReceiptScreen } from "@point_of_sale/app/screens/receipt_screen/receipt_screen";
import { patch } from "@web/core/utils/patch";
import { rpc } from "@web/core/network/rpc";

/*
 * Native Print for POS receipts.
 *
 * POS receipts are not ir.actions.report reports, so the report handler
 * cannot see them. But since Odoo's own printer refactor (17+) the
 * receipt is a standalone component rendered in the page, and Odoo's
 * IoT path itself prints an IMAGE of that receipt (htmlToImg). We do
 * exactly the same thing the user asked for:
 *
 *   receipt DOM -> PNG image (this file) -> window.__NATIVE_PRINT__
 *   .printImage -> the native app scales/thresholds the image to the
 *   receipt printer's width and prints it as an ESC/POS raster.
 *
 * Opt-in and fail-safe, like everything in this addon:
 * - No bridge (normal browser): untouched default POS printing.
 * - No server rule for source 'point_of_sale.receipt': default printing.
 * - Any capture/queue error: default printing (the error is logged).
 * - Basic receipts keep the default flow (their layout differs from the
 *   on-screen full receipt we capture).
 */

const POS_RECEIPT_SOURCE = "point_of_sale.receipt";

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

// Images and canvases must be self-contained before the receipt DOM is
// serialised into an SVG image, or the canvas would taint/blank them.
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
            // A tainted canvas (cross-origin) is left as-is; the native
            // side still prints the rest of the receipt.
        }
    }
}

// DOM -> PNG via SVG foreignObject (the same family of technique as
// Odoo's own htmlToImg). Returns a Uint8Array of PNG bytes.
async function elementToPngBytes(element, scale = 2) {
    const width = element.offsetWidth;
    const height = element.offsetHeight;
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
    return new Uint8Array(await blob.arrayBuffer());
}

async function printPosReceiptNatively(screen) {
    const bridge = nativeBridge();
    if (!bridge || typeof bridge.printImage !== "function") {
        return false;
    }
    const resolved = await rpc("/native_print/rules", {
        report_name: POS_RECEIPT_SOURCE,
    });
    if (!resolved || !resolved.rule) {
        return false; // Not configured for POS receipts: default flow.
    }
    const element = findReceiptElement();
    if (!element) {
        return false;
    }
    const order =
        screen.currentOrder ||
        (screen.pos && screen.pos.getOrder ? screen.pos.getOrder() : null);
    const orderRef =
        (order && (order.name || order.pos_reference || order.uid)) ||
        "pos-order";
    const bytes = await elementToPngBytes(element, 2);
    const idempotencyKey =
        `pos-${orderRef}-${Date.now()}-` +
        Math.random().toString(36).slice(2, 10);
    // The bridge returns once the image job is durably queued.
    await bridge.printImage({
        idempotencyKey,
        reportName: POS_RECEIPT_SOURCE,
        printerPattern: resolved.rule.printer_pattern,
        paper: resolved.rule.paper || "80mm",
        bytes,
    });
    // Server-side job log (best effort; the app's queue is the truth).
    rpc("/native_print/job_status", {
        idempotency_key: idempotencyKey,
        state: "queued",
        report_name: POS_RECEIPT_SOURCE,
        byte_size: bytes.length,
    }).catch(() => {});
    if (order) {
        order._printed = true; // Same flag the default flow sets.
    }
    return true;
}

patch(ReceiptScreen.prototype, {
    async printReceipt() {
        const opts = arguments[0];
        const isBasic = !!(opts && (opts.basic || opts.basicReceipt));
        if (!isBasic) {
            try {
                if (await printPosReceiptNatively(this)) {
                    return;
                }
            } catch (error) {
                console.warn(
                    "Native Print: POS receipt capture failed; " +
                        "falling back to default POS printing.",
                    error,
                );
            }
        }
        return super.printReceipt(...arguments);
    },
});

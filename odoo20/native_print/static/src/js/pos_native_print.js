/** @odoo-module **/
import { PosTicketPrinterPlugin } from "@point_of_sale/app/plugins/pos_ticket_printer_plugin";
import { patch } from "@web/core/utils/patch";
import { rpc } from "@web/core/network/rpc";
import { _t } from "@web/core/l10n/translation";
import {
    JOB_TERMINAL_STATES as TERMINAL_STATES,
    normalizeBridgeJobState,
    notifyJobUpdated,
    notifyUser,
    recoveryButtons,
    resolveBridgeJobId,
    resolveBridgeTarget,
    safeErrorCode,
} from "./native_print_client";

const POS_RECEIPT_SOURCE = "point_of_sale.receipt";
const RECEIPT_LAYOUT_FIX_CSS =
    ".pos-receipt{padding-bottom:16px !important}" +
    ".pos-receipt,.pos-receipt *{float:none !important;position:static !important;line-height:1.35 !important;box-sizing:border-box !important}" +
    ".pos-receipt *:not(img):not(canvas):not(svg){height:auto !important;min-height:0 !important;max-height:none !important;block-size:auto !important;overflow:visible !important}" +
    ".pos-receipt span:not(.pos-receipt-right-align),.pos-receipt .cashier,.pos-receipt .cashier>div{width:auto !important;inline-size:auto !important}" +
    ".pos-receipt .pos-receipt-amount,.pos-receipt .paymentlines,.pos-receipt .pos-receipt-taxes>div{display:flex !important;justify-content:space-between !important;gap:0.75em !important;align-items:baseline !important}" +
    ".pos-receipt .ms-auto{margin-inline-start:auto !important;white-space:nowrap !important;flex-shrink:0 !important}" +
    ".pos-receipt .pos-receipt-contact,.pos-receipt .pos-receipt-customer{display:flex !important;flex-direction:column !important;align-items:center !important}" +
    ".pos-receipt .pos-receipt-right-align{display:inline-block !important;margin-inline-start:auto !important;text-align:end !important;white-space:nowrap !important;flex-shrink:0 !important}" +
    ".pos-receipt .pos-receipt-qr{display:flex !important;flex-direction:row !important;align-items:flex-start !important;gap:0.75em !important}" +
    ".pos-receipt .pos-receipt-qr>div:first-child{flex:0 0 auto !important;width:auto !important;inline-size:auto !important}" +
    ".pos-receipt .pos-receipt-qr>div:last-child{flex:1 1 0% !important;min-width:0 !important;width:auto !important;inline-size:auto !important}" +
    ".pos-receipt .portal-url,.pos-receipt .text-break{overflow-wrap:anywhere !important;word-break:break-word !important;max-width:100% !important}";

(function installReceiptLayoutFix() {
    if (!nativeBridge()) return;
    if (typeof document === "undefined" || !document.head || typeof document.createElement !== "function") return;
    if (typeof document.getElementById === "function" && document.getElementById("np-receipt-layout-fix")) return;
    const style = document.createElement("style");
    style.id = "np-receipt-layout-fix";
    style.textContent = RECEIPT_LAYOUT_FIX_CSS;
    document.head.appendChild(style);
})();

export const POS_CAPTURE_TIMEOUT_MS = 15_000;
const inFlightPosKeys = new Set();

function nativeBridge() {
    return (typeof window !== "undefined" && window.__NATIVE_PRINT__) || null;
}

function isNativeApp() {
    return Boolean(nativeBridge());
}

function notificationEnv(contextObj) {
    if (contextObj?.notification && typeof contextObj.notification.add === "function") {
        return { services: { notification: contextObj.notification } };
    }
    if (contextObj?.services?.notification && typeof contextObj.services.notification.add === "function") {
        return { services: { notification: contextObj.services.notification } };
    }
    if (contextObj?.pos?.env?.services?.notification) {
        return contextObj.pos.env;
    }
    if (contextObj?.pos?.services?.notification && typeof contextObj.pos.services.notification.add === "function") {
        return { services: { notification: contextObj.pos.services.notification } };
    }
    if (contextObj?.pos?.notification && typeof contextObj.pos.notification.add === "function") {
        return { services: { notification: contextObj.pos.notification } };
    }
    if (contextObj?.env?.services?.notification) {
        return contextObj.env;
    }
    return contextObj?.pos?.env || contextObj?.env || null;
}

function notifyPosNativeFailure(contextObj, reason) {
    let message = _t("Could not print this receipt. Check the printer and try again.");
    if (reason) {
        message += " " + reason;
    }
    notifyUser(notificationEnv(contextObj), message, "warning", undefined, recoveryButtons());
}

const posFailNoPrinter = () => _t("Reason: no printer is available on this computer.");
const posFailCapture = () => _t("Reason: the receipt could not be captured as an image.");

function extractReceiptText(element) {
    if (!element) {
        return "";
    }
    const raw =
        (typeof element.innerText === "string" && element.innerText) ||
        element.textContent ||
        "";
    return raw
        .replace(/\r\n?/g, "\n")
        .split("\n")
        .map((line) => line.replace(/[ \t]+/g, " ").trimEnd())
        .join("\n")
        .replace(/\n{3,}/g, "\n\n")
        .trim();
}

async function canvasToPngBytes(canvas) {
    if (canvas && typeof canvas.toBlob === "function") {
        const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
        if (blob) {
            return Array.from(new Uint8Array(await blob.arrayBuffer()));
        }
    }
    if (canvas && typeof canvas.toDataURL === "function") {
        const dataUrl = canvas.toDataURL("image/png");
        const idx = dataUrl.indexOf(",");
        if (idx > 0) {
            const bin = atob(dataUrl.slice(idx + 1));
            const out = new Array(bin.length);
            for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
            return out;
        }
    }
    throw new Error("receipt canvas produced no image");
}

function extractCashierUserId(cashierObj, posStore, order) {
    const idOf = (v) => {
        if (v == null) return null;
        if (typeof v === "number" || typeof v === "string") return v;
        if (Array.isArray(v)) return v[0] ?? null;
        if (typeof v === "object") return v.id ?? null;
        return null;
    };
    const raw =
        idOf(cashierObj) ||
        idOf(posStore && posStore.user) ||
        idOf(posStore && posStore.session && posStore.session.user_id) ||
        idOf(order && order.user_id) ||
        0;
    return String(raw).replace(/[^0-9]/g, "") || "0";
}

function posReceiptKey(safeRef, userId) {
    // Reuse the in-flight key within 15s for double-fire dedupe; persist per business day
    // in localStorage (falling back to sessionStorage) so tab reloads never reset attempt to 1.
    const now = Date.now();
    const day = new Date(now).toISOString().slice(0, 10).replace(/-/g, "");
    const storeKey = `np.pos.key.${day}.${safeRef}.u${userId}`;
    let raw = null;
    try {
        if (typeof localStorage !== "undefined" && localStorage) {
            raw = localStorage.getItem(storeKey);
        }
    } catch (_err) { /* fall back to sessionStorage */ }
    if (raw == null) {
        try {
            if (typeof sessionStorage !== "undefined" && sessionStorage) {
                raw = sessionStorage.getItem(storeKey);
            }
        } catch (_err) { /* storage unavailable */ }
    }
    let record = null;
    try {
        record = JSON.parse(raw || "null");
    } catch (_err) { /* ignore corrupt entry */ }
    if (record && record.key && now - (record.ts || 0) < 15000) {
        return record.key;
    }
    const attempt = (record && record.attempt ? record.attempt : 0) + 1;
    const key = `pos-${safeRef}-${day}-u${userId}-a${attempt}`;
    const payload = JSON.stringify({ key, ts: now, attempt });
    let saved = false;
    try {
        if (typeof localStorage !== "undefined" && localStorage) {
            if (typeof localStorage.length === "number" && typeof localStorage.key === "function") {
                const cutoff = new Date(now - 2 * 86400000).toISOString().slice(0, 10).replace(/-/g, "");
                for (let i = localStorage.length - 1; i >= 0; i--) {
                    const k = localStorage.key(i);
                    const m = k && /^np\.pos\.key\.(\d{8})\./.exec(k);
                    if (m && m[1] < cutoff) {
                        localStorage.removeItem(k);
                    }
                }
            }
            localStorage.setItem(storeKey, payload);
            saved = true;
        }
    } catch (_err) { /* fall back to sessionStorage */ }
    if (!saved) {
        try {
            if (typeof sessionStorage !== "undefined" && sessionStorage) {
                sessionStorage.setItem(storeKey, payload);
            }
        } catch (_err) { /* ignore */ }
    }
    return key;
}

function finishPosReceiptAttempt(safeRef, userId) {
    // Keep the attempt counter as a tombstone so the next deliberate reprint gets a fresh key.
    const day = new Date().toISOString().slice(0, 10).replace(/-/g, "");
    const storeKey = `np.pos.key.${day}.${safeRef}.u${userId}`;
    let raw = null;
    try {
        if (typeof localStorage !== "undefined" && localStorage) {
            raw = localStorage.getItem(storeKey);
        }
    } catch (_err) { /* fall back to sessionStorage */ }
    if (raw == null) {
        try {
            if (typeof sessionStorage !== "undefined" && sessionStorage) {
                raw = sessionStorage.getItem(storeKey);
            }
        } catch (_err) { /* ignore */ }
    }
    let record = null;
    try {
        record = JSON.parse(raw || "null");
    } catch (_err) { /* ignore */ }
    const payload = JSON.stringify({
        key: null, ts: 0, attempt: record && record.attempt ? record.attempt : 1,
    });
    let saved = false;
    try {
        if (typeof localStorage !== "undefined" && localStorage) {
            localStorage.setItem(storeKey, payload);
            saved = true;
        }
    } catch (_err) { /* fall back to sessionStorage */ }
    if (!saved) {
        try {
            if (typeof sessionStorage !== "undefined" && sessionStorage) {
                sessionStorage.setItem(storeKey, payload);
            }
        } catch (_err) { /* ignore */ }
    }
}

async function resolveLivePosTarget(bridge) {
    const opts = { isPos: true, preferClasses: ["thermal", "label"] };
    let target = await resolveBridgeTarget(bridge, opts).catch(() => null);
    if (!target) {
        await new Promise((resolve) => setTimeout(resolve, 250));
        target = await resolveBridgeTarget(bridge, { ...opts, force: true }).catch(() => null);
    }
    return target;
}

async function watchPosJob(bridge, jobId, rule, byteSize, safeRef, printerName, env, userId, order, bridgeJobId = null) {
    const pollJobId = bridgeJobId || jobId;
    const normState = (s) => (typeof normalizeBridgeJobState === "function" ? normalizeBridgeJobState(s) : s);
    const absoluteDeadline = Date.now() + 30 * 60 * 1000;
    let lastProgressAt = Date.now();
    let lastSeenState = "queued";
    let lastSeenAttempts = 1;
    let reachedTerminal = false;
    let consecutiveErrors = 0;
    let consecutiveMissing = 0;
    let lastOutcomeKey = null;
    let orderReleased = false;
    const syncJobStatus = async (state, errorCode, attempts) => {
        await rpc(
            "/native_print/job_status",
            {
                idempotency_key: jobId,
                state,
                error_code: errorCode || null,
                device: printerName || rule.printer_pattern,
                report_name: POS_RECEIPT_SOURCE,
                source_type: "pos_receipt",
                copies: rule.copies || 1,
                mode: rule.mode || "gdi",
                pages: 1,
                byte_size: byteSize,
                attempts,
            },
            { silent: true },
        ).catch(() => {});
    };

    for (let i = 0; Date.now() < absoluteDeadline && Date.now() - lastProgressAt < 5 * 60 * 1000; i++) {
        await new Promise((resolve) =>
            setTimeout(resolve, i === 0 ? 50 : (i < 4 ? 150 : (i < 12 ? 400 : 1500))));
        try {
            let state = null;
            let errorCode = null;
            let attempts = lastSeenAttempts;
            if (typeof bridge.jobDetail === "function") {
                const detail = await bridge.jobDetail(pollJobId);
                if (detail) {
                    state = normState(detail.state);
                    errorCode = detail.errorCode || detail.error_code || null;
                    attempts = detail.attempts || attempts;
                }
            } else if (typeof bridge.printStatus === "function") {
                state = normState(await bridge.printStatus(pollJobId));
            }
            consecutiveErrors = 0;
            if (!state) {
                if (++consecutiveMissing >= 15) {
                    break;
                }
                continue;
            }
            consecutiveMissing = 0;
            const progressed = state !== lastSeenState || attempts !== lastSeenAttempts;
            if (progressed) {
                lastProgressAt = Date.now();
            }
            if (!TERMINAL_STATES.has(state)) {
                if (progressed) {
                    lastSeenState = state;
                    lastSeenAttempts = attempts;
                    await syncJobStatus(state, errorCode, attempts);
                }
                continue;
            }
            // Quiet retry check: avoid flashing a red toast if the worker already re-queued.
            if (state === "failed" && (state + ":" + attempts) !== lastOutcomeKey && typeof bridge.jobDetail === "function") {
                await new Promise((resolve) => setTimeout(resolve, 900));
                const recheck = await bridge.jobDetail(pollJobId).catch(() => null);
                const recheckState = recheck && normState(recheck.state);
                if (recheckState && !TERMINAL_STATES.has(recheckState)) {
                    lastSeenState = recheckState;
                    lastSeenAttempts = recheck.attempts || attempts;
                    lastProgressAt = Date.now();
                    await syncJobStatus(
                        recheckState,
                        recheck.errorCode || recheck.error_code || null,
                        lastSeenAttempts,
                    );
                    continue;
                }
                if (recheckState) {
                    state = recheckState;
                    errorCode = recheck.errorCode || recheck.error_code || errorCode;
                    attempts = recheck.attempts || attempts;
                }
            }
            lastSeenState = state;
            lastSeenAttempts = attempts;
            const outcomeKey = state + ":" + attempts;
            if (outcomeKey === lastOutcomeKey) {
                continue;
            }
            lastOutcomeKey = outcomeKey;
            reachedTerminal = true;
            if (!orderReleased) {
                orderReleased = true;
                if (safeRef) {
                    finishPosReceiptAttempt(safeRef, userId);
                }
            }
            if (order) {
                if (state === "done") {
                    order._printed = true;
                } else if (state === "failed" || state === "dead_letter" || state === "canceled") {
                    order._printed = false;
                }
            }
            await syncJobStatus(state, errorCode, attempts);
            notifyJobUpdated();
            if (state === "done") {
                notifyUser(
                    env,
                    _t('Sent "%s" to %s', _t("POS Receipt"), printerName || rule.printer_pattern),
                    "success",
                );
                break;
            }
            if (state === "failed" || state === "dead_letter") {
                notifyUser(
                    env,
                    _t('Receipt printing failed on "%s" (%s). Check the printer before reprinting.', printerName || rule.printer_pattern, safeErrorCode(errorCode, state)),
                    "danger",
                    undefined,
                    typeof recoveryButtons === "function" ? recoveryButtons() : null,
                );
                if (state === "dead_letter") {
                    break;
                }
                continue;
            }
            break;
        } catch (_err) {
            if (++consecutiveErrors >= 10) {
                break;
            }
        }
    }
    if (!reachedTerminal) {
        if (!orderReleased && safeRef) {
            orderReleased = true;
            finishPosReceiptAttempt(safeRef, userId);
        }
        notifyUser(
            env,
            _t('Receipt job on "%s" is still running. Check Print Jobs for the final status.', printerName || rule.printer_pattern),
            "warning",
            undefined,
            typeof recoveryButtons === "function" ? recoveryButtons() : null,
        );
    }
}

function postPosDeclined(key, code, safeRef, userId, deviceName, mode) {
    if (!key) {
        return;
    }
    const params = {
        idempotency_key: key,
        state: "failed",
        error_code: code,
        report_name: POS_RECEIPT_SOURCE,
        source_type: "pos_receipt",
        copies: 1,
        pages: 1,
        byte_size: 0,
    };
    if (deviceName) {
        params.device = deviceName;
    }
    if (mode) {
        params.mode = mode;
    }
    Promise.resolve(rpc("/native_print/job_status", params, { silent: true }))
        .catch(() => {})
        .then(() => notifyJobUpdated());
    if (safeRef) {
        finishPosReceiptAttempt(safeRef, userId);
    }
}

function removeIframe(iframe) {
    try {
        if (typeof iframe?.remove === "function") {
            iframe.remove();
        } else if (iframe?.parentNode && typeof iframe.parentNode.removeChild === "function") {
            iframe.parentNode.removeChild(iframe);
        }
    } catch (_err) {
        // Best-effort DOM cleanup.
    }
}

function withTimeout(promise, ms, label, onLateResolve = null) {
    let settled = false;
    let timer = null;
    return new Promise((resolve, reject) => {
        timer = setTimeout(() => {
            if (!settled) {
                settled = true;
                reject(new Error(`${label} timed out after ${ms}ms`));
            }
        }, ms);
        Promise.resolve(promise).then(
            (value) => {
                clearTimeout(timer);
                if (!settled) {
                    settled = true;
                    resolve(value);
                } else if (typeof onLateResolve === "function") {
                    onLateResolve(value);
                }
            },
            (err) => {
                clearTimeout(timer);
                if (!settled) {
                    settled = true;
                    reject(err);
                }
            },
        );
    });
}

const RECEIPT_TEMPLATE = "point_of_sale.pos_order_receipt";

function isIframeLike(value) {
    return Boolean(
        value &&
        (String(value.tagName || "").toUpperCase() === "IFRAME" ||
            value.contentDocument ||
            typeof value.querySelector === "function"),
    );
}

// Only retry the other call shape when the failure looks like an API
// mismatch. A genuine rendering/runtime error must not call the plugin
// twice (it may already have mutated the POS state).
function isSignatureError(error) {
    if (error instanceof TypeError) {
        return true;
    }
    return /not a function|not iterable|cannot read|undefined|argument|arity|template|props/i.test(
        String(error?.message || error || ""),
    );
}

async function renderReceiptIframe(plugin, receiptProps) {
    if (typeof plugin?.generateIframe !== "function") {
        throw new Error("generateIframe unavailable");
    }
    // Odoo 20 core: generateIframe(template, data) renders the template
    // with `data` produced by getOrderReceiptData(). Passing the raw
    // {order, basic, simplified} object makes the QWeb template render
    // an empty #pos-receipt, so generateImage() later finds nothing.
    let data = receiptProps;
    if (typeof plugin.getOrderReceiptData === "function") {
        try {
            data = await plugin.getOrderReceiptData(receiptProps.order, {
                basic: !!receiptProps.basic,
                simplified: !!receiptProps.simplified,
            });
        } catch (error) {
            console.warn("Native Print: Odoo 20 receipt data generation failed.", error);
        }
    }
    const shapes = [
        () => plugin.generateIframe(RECEIPT_TEMPLATE, data),
        () => plugin.generateIframe(receiptProps),
    ];
    let lastError = null;
    for (let i = 0; i < shapes.length; i++) {
        try {
            const result = await Promise.resolve(shapes[i]());
            const iframe = result?.el || result?.iframe || result;
            if (isIframeLike(iframe)) {
                return iframe;
            }
            removeIframe(iframe);
            lastError = new Error("generateIframe returned no iframe");
        } catch (error) {
            lastError = error;
            if (i === 0 && !isSignatureError(error)) {
                throw error;
            }
        }
    }
    throw lastError || new Error("generateIframe failed");
}

async function generateReceiptCanvas(plugin, iframe, receiptEl) {
    if (typeof plugin?.generateImage !== "function") {
        throw new Error("generateImage unavailable");
    }
    const shapes = [
        () => plugin.generateImage(iframe),
        () => plugin.generateImage(iframe, receiptEl),
    ];
    let lastError = null;
    for (let i = 0; i < shapes.length; i++) {
        try {
            const result = await Promise.resolve(shapes[i]());
            const canvas = result?.canvas || result;
            if (
                canvas &&
                (typeof canvas.toBlob === "function" ||
                    typeof canvas.toDataURL === "function" ||
                    (canvas.width || 0) > 0)
            ) {
                return canvas;
            }
            lastError = new Error("generateImage returned an empty canvas");
        } catch (error) {
            lastError = error;
            if (i === 0 && !isSignatureError(error)) {
                throw error;
            }
        }
    }
    throw lastError || new Error("generateImage failed");
}

function findVisibleReceiptElement() {
    if (typeof document === "undefined" || typeof document.querySelector !== "function") {
        return null;
    }
    for (const selector of ["#pos-receipt", ".pos-receipt-container .pos-receipt", ".pos-receipt"]) {
        try {
            const el = document.querySelector(selector);
            if (el && (el.offsetWidth || el.offsetHeight || el.getBoundingClientRect?.().width)) {
                return el;
            }
        } catch (_err) {
            /* keep trying the next selector */
        }
    }
    return null;
}

// Last-resort capture of the receipt Odoo already rendered on screen.
// The clone carries inlined computed styles; external url() references
// are stripped so the SVG cannot taint the canvas in WebView2.
async function visibleReceiptToPngBytes(element, scale = 2) {
    const width = element.offsetWidth || element.getBoundingClientRect?.().width || 300;
    let height = Math.max(
        element.offsetHeight || 0,
        element.scrollHeight || 0,
        Math.ceil(element.getBoundingClientRect?.().height || 0),
    ) || 400;
    if (!width || !height) {
        throw new Error("visible receipt has no size");
    }
    const clone = element.cloneNode(true);
    const inlineStyles = (source, target) => {
        try {
            const computed = window.getComputedStyle(source);
            let css = "";
            for (let i = 0; i < computed.length; i++) {
                const prop = computed[i];
                css += `${prop}:${computed.getPropertyValue(prop)};`;
            }
            target.setAttribute(
                "style",
                css.replace(
                    /url\(\s*(?:"(?!data:)[^"]*"|'(?!data:)[^']*'|(?![\s"']*data:)[^)]+)\s*\)/gi,
                    "none",
                ),
            );
        } catch (_err) {
            /* keep the clone's original classes/styles */
        }
        const sourceKids = source.children || [];
        const targetKids = target.children || [];
        for (let i = 0; i < sourceKids.length && i < targetKids.length; i++) {
            inlineStyles(sourceKids[i], targetKids[i]);
        }
    };
    inlineStyles(element, clone);
    // Float + `text-align: start` + frozen computed pixel heights stack/overlap
    // wrapped lines in <foreignObject>; reset box heights and enforce flex rows.
    const layoutFix = typeof document.createElementNS === "function"
        ? document.createElementNS("http://www.w3.org/1999/xhtml", "style")
        : document.createElement("style");
    layoutFix.textContent = RECEIPT_LAYOUT_FIX_CSS;
    if (typeof clone.insertBefore === "function") {
        clone.insertBefore(layoutFix, clone.firstChild);
    } else if (typeof clone.appendChild === "function") {
        clone.appendChild(layoutFix);
    }
    const inlineUrlToDataUrl = async (url, fallbackImg) => {
        if (!url || url.startsWith("data:")) return url || null;
        try {
            if (typeof fetch === "function") {
                const resp = await fetch(url, { credentials: "same-origin" });
                if (resp && resp.ok) {
                    const blob = await resp.blob();
                    const dataUrl = await new Promise((resolve) => {
                        const fr = new FileReader();
                        fr.onload = () => resolve(typeof fr.result === "string" ? fr.result : null);
                        fr.onerror = () => resolve(null);
                        fr.readAsDataURL(blob);
                    });
                    if (dataUrl && dataUrl.startsWith("data:image/")) return dataUrl;
                }
            }
        } catch (_err) { /* try canvas fallback */ }
        try {
            if (fallbackImg && fallbackImg.naturalWidth > 0 && fallbackImg.naturalHeight > 0) {
                const c = document.createElement("canvas");
                c.width = fallbackImg.naturalWidth;
                c.height = fallbackImg.naturalHeight;
                c.getContext("2d").drawImage(fallbackImg, 0, 0);
                const d = c.toDataURL("image/png");
                if (d && d.startsWith("data:image/")) return d;
            }
        } catch (_err) { /* cross-origin canvas */ }
        return null;
    };
    // Inline <img> elements (company logos, <img> barcodes/QR) and <canvas> for <foreignObject>.
    if (
        typeof element.querySelectorAll === "function" &&
        typeof clone.querySelectorAll === "function" &&
        typeof document !== "undefined" &&
        typeof document.createElement === "function"
    ) {
        const TRANSPARENT_PIXEL = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==";
        const sourceImgs = element.querySelectorAll("img");
        const cloneImgs = clone.querySelectorAll("img");
        await Promise.all(Array.from({ length: Math.min(sourceImgs.length, cloneImgs.length) }, async (_, i) => {
            cloneImgs[i].removeAttribute?.("srcset");
            cloneImgs[i].removeAttribute?.("loading");
            cloneImgs[i].removeAttribute?.("decoding");
            const src = sourceImgs[i].currentSrc || sourceImgs[i].src;
            if (src && !src.startsWith("data:")) {
                const dataUrl = await inlineUrlToDataUrl(src, sourceImgs[i]);
                cloneImgs[i].setAttribute("src", dataUrl || TRANSPARENT_PIXEL);
            }
        }));
        const sourceCanvases = element.querySelectorAll("canvas");
        const cloneCanvases = clone.querySelectorAll("canvas");
        for (let i = 0; i < sourceCanvases.length && i < cloneCanvases.length; i++) {
            try {
                if (!sourceCanvases[i].width || !sourceCanvases[i].height) continue;
                const dataUrl = sourceCanvases[i].toDataURL("image/png");
                if (!dataUrl || !dataUrl.startsWith("data:image/")) continue;
                const img = document.createElement("img");
                img.setAttribute("src", dataUrl);
                img.setAttribute("width", String(sourceCanvases[i].width));
                img.setAttribute("height", String(sourceCanvases[i].height));
                const canvasStyle = cloneCanvases[i].getAttribute?.("style");
                if (canvasStyle) img.setAttribute("style", canvasStyle);
                cloneCanvases[i].replaceWith(img);
            } catch (_err) {
                // Leave cross-origin canvas as-is.
            }
        }
    }
    const rootDir =
        element.getAttribute?.("dir") ||
        (typeof window !== "undefined" && window.getComputedStyle && window.getComputedStyle(element)?.direction) ||
        (typeof document !== "undefined" && document.documentElement?.dir) ||
        "";
    if (rootDir) clone.setAttribute("dir", rootDir);
    const rootLang =
        element.getAttribute?.("lang") ||
        (typeof document !== "undefined" && document.documentElement?.lang) ||
        "";
    if (rootLang) clone.setAttribute("lang", rootLang);
    clone.setAttribute(
        "style",
        (clone.getAttribute?.("style") || "") +
            ";margin:0 !important;position:static !important;transform:none !important;height:auto !important;min-height:0 !important;max-height:none !important;overflow:visible !important;box-sizing:border-box !important;",
    );
    if (typeof document !== "undefined" && document.body && typeof document.createElement === "function") {
        const probe = document.createElement("div");
        probe.setAttribute("style", `position:fixed;left:-99999px;top:0;visibility:hidden;pointer-events:none;width:${width}px;margin:0;padding:0;`);
        probe.appendChild(clone);
        document.body.appendChild(probe);
        try {
            const probed = Math.max(clone.scrollHeight || 0, clone.offsetHeight || 0);
            if (probed > 0) {
                height = Math.ceil(Math.max(height, probed) * 1.03) + 24;
            }
        } finally {
            probe.remove();
        }
    }
    const effectiveScale = Math.max(0.5, Math.min(scale, 14000 / height, 4096 / width));
    const scaledW = Math.max(1, Math.ceil(width * effectiveScale));
    const scaledH = Math.max(1, Math.ceil(height * effectiveScale));
    clone.setAttribute("xmlns", "http://www.w3.org/1999/xhtml");
    const serialized = new XMLSerializer()
        .serializeToString(clone)
        .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F]/g, "");
    const svg =
        `<svg xmlns="http://www.w3.org/2000/svg" width="${scaledW}" height="${scaledH}" viewBox="0 0 ${width} ${height}">` +
        `<foreignObject width="${width}" height="${height}">${serialized}</foreignObject></svg>`;
    const image = new Image();
    const loaded = new Promise((resolve, reject) => {
        const timer = setTimeout(
            () => reject(new Error("visible receipt SVG timed out")),
            POS_CAPTURE_TIMEOUT_MS,
        );
        image.onload = () => {
            clearTimeout(timer);
            resolve();
        };
        image.onerror = () => {
            clearTimeout(timer);
            reject(new Error("visible receipt SVG failed to load"));
        };
    });
    image.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg);
    await loaded;
    const canvas = document.createElement("canvas");
    canvas.width = scaledW;
    canvas.height = scaledH;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, scaledW, scaledH);
    ctx.imageSmoothingEnabled = true;
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(image, 0, 0, scaledW, scaledH);
    return canvasToPngBytes(canvas);
}

async function textReceiptToPngBytes(text) {
    if (typeof document === "undefined" || typeof document.createElement !== "function") {
        throw new Error("canvas unavailable");
    }
    const FONT_PX = 14;
    const LINE_H = 20;
    const PAD = 16;
    const WIDTH = 384;
    const canvas = document.createElement("canvas");
    const ctx = canvas.getContext?.("2d");
    if (!ctx) {
        throw new Error("2d context unavailable");
    }
    ctx.font = FONT_PX + "px 'Courier New', monospace";
    const charW = ctx.measureText?.("M")?.width || 8.4;
    const maxChars = Math.max(16, Math.floor((WIDTH - PAD * 2) / charW));
    const lines = [];
    for (const rawLine of String(text).replace(/\f/g, "\n").split("\n")) {
        const line = rawLine.replace(/\r$/, "").replace(/\t/g, "    ");
        if (line.length <= maxChars) {
            lines.push(line);
        } else {
            for (let i = 0; i < line.length; i += maxChars) {
                lines.push(line.slice(i, i + maxChars));
            }
        }
    }
    const height = Math.min(16000, PAD * 2 + Math.max(1, lines.length) * LINE_H);
    canvas.width = WIDTH;
    canvas.height = height;
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, WIDTH, height);
    ctx.fillStyle = "#000000";
    ctx.font = FONT_PX + "px 'Courier New', monospace";
    ctx.textBaseline = "top";
    lines.forEach((line, idx) => {
        const y = PAD + idx * LINE_H;
        if (y < height - LINE_H) ctx.fillText(line, PAD, y);
    });
    return canvasToPngBytes(canvas);
}

async function printPosReceiptNatively(plugin, { order, basic = false, simplified = false, skipMarkPrinted = false }) {
    const bridge = nativeBridge();
    const posStore = plugin?.pos || plugin || null;
    const keyRef =
        (order && (order.uuid || order.uid || order.name || order.pos_reference)) ||
        `pos-html-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    const safeRef = String(keyRef).replace(/[^A-Za-z0-9_-]/g, "_").slice(0, 40);
    const cashierObj =
        posStore?.getCashier?.() ||
        posStore?.get_cashier?.() ||
        posStore?.cashier ||
        null;
    const userId = extractCashierUserId(cashierObj, posStore, order);
    const idempotencyKey = posReceiptKey(safeRef, userId);

    if (!bridge || typeof bridge.printImage !== "function") {
        return { reason: posFailNoPrinter(), code: "POS_NO_PRINTER", key: idempotencyKey, safeRef, userId };
    }
    if (inFlightPosKeys.has(idempotencyKey)) {
        return { deduped: true };
    }
    inFlightPosKeys.add(idempotencyKey);
    try {
        const target = await resolveLivePosTarget(bridge);
        if (!target) {
            return { reason: posFailNoPrinter(), code: "POS_NO_PRINTER", key: idempotencyKey, safeRef, userId };
        }
        const rule = target.rule;
        const effectiveMode = rule.mode || "gdi";
        let bytes = null;
        let fallbackText = null;
        let sourceWidthMm;
        let iframe = null;
        let receiptEl = null;
        try {
            iframe = await withTimeout(
                renderReceiptIframe(plugin, { order, basic, simplified }),
                POS_CAPTURE_TIMEOUT_MS,
                "generateIframe",
                removeIframe,
            );
            receiptEl =
                iframe?.contentDocument?.getElementById?.("pos-receipt") ||
                iframe?.contentDocument?.body ||
                null;
            try {
                const canvas = await withTimeout(
                    generateReceiptCanvas(plugin, iframe, receiptEl),
                    POS_CAPTURE_TIMEOUT_MS,
                    "generateImage",
                );
                bytes = await withTimeout(
                    canvasToPngBytes(canvas),
                    POS_CAPTURE_TIMEOUT_MS,
                    "canvasToPngBytes",
                );
                if (!bytes?.length) {
                    throw new Error("receipt PNG is empty");
                }
                const widthPx = receiptEl?.offsetWidth || canvas?.width || 0;
                if (widthPx) {
                    sourceWidthMm = Math.round(((widthPx * 25.4) / 96) * 10) / 10;
                }
            } catch (captureError) {
                console.warn("Native Print: Odoo 20 receipt image capture failed.", captureError);
                fallbackText = extractReceiptText(receiptEl);
            }
        } catch (iframeError) {
            console.warn("Native Print: Odoo 20 receipt iframe generation failed.", iframeError);
        } finally {
            removeIframe(iframe);
            iframe = null;
        }

        // Prefer capturing the on-screen receipt as PNG before falling back to plain text.
        if (!bytes) {
            const visible = findVisibleReceiptElement();
            if (visible) {
                try {
                    bytes = await withTimeout(
                        visibleReceiptToPngBytes(visible),
                        POS_CAPTURE_TIMEOUT_MS,
                        "visibleReceiptToPngBytes",
                    );
                    if (bytes?.length) {
                        const widthPx = visible.offsetWidth || 300;
                        sourceWidthMm = Math.round(((widthPx * 25.4) / 96) * 10) / 10;
                        fallbackText = null;
                    } else {
                        bytes = null;
                    }
                } catch (visibleError) {
                    console.warn("Native Print: on-screen receipt capture failed.", visibleError);
                }
                if (!bytes && !fallbackText) {
                    fallbackText = extractReceiptText(visible);
                }
            }
        }
        // Only ESC/POS printers accept raw text; rasterize text to PNG for GDI/virtual/label printers.
        if (!bytes && fallbackText && (effectiveMode !== "escpos" || typeof bridge.printText !== "function")) {
            try {
                const pngBytes = await withTimeout(
                    textReceiptToPngBytes(fallbackText),
                    POS_CAPTURE_TIMEOUT_MS,
                    "textReceiptToPngBytes",
                );
                if (pngBytes?.length) {
                    bytes = pngBytes;
                    if (!sourceWidthMm) {
                        sourceWidthMm = 80;
                    }
                }
            } catch (textRenderError) {
                console.warn("Native Print: text-to-PNG receipt fallback failed.", textRenderError);
            }
            fallbackText = null;
        }
        if (!bytes && !fallbackText) {
            return {
                reason: posFailCapture(),
                code: "POS_CAPTURE_FAILED",
                key: idempotencyKey,
                safeRef,
                userId,
                device: target.printer.name,
                mode: effectiveMode,
            };
        }

        const payloadByteSize = fallbackText
            ? new TextEncoder().encode(fallbackText).length
            : bytes.length;
        let bridgeJobId = idempotencyKey;
        try {
            let enqueueRes;
            if (fallbackText) {
                enqueueRes = await bridge.printText({
                    idempotencyKey,
                    reportName: POS_RECEIPT_SOURCE,
                    printer: target.printer.name,
                    printerPattern: rule.printer_pattern,
                    text: fallbackText,
                    cut: true,
                    copies: rule.copies || 1,
                });
            } else {
                enqueueRes = await bridge.printImage({
                    idempotencyKey,
                    reportName: POS_RECEIPT_SOURCE,
                    printer: target.printer.name,
                    printerPattern: rule.printer_pattern,
                    mode: effectiveMode,
                    mediaRollWidthMm: (rule.media && rule.media.roll_width_mm) || undefined,
                    sourceWidthMm,
                    copies: rule.copies || 1,
                    bytes,
                });
            }
            bridgeJobId = resolveBridgeJobId(enqueueRes, idempotencyKey);
        } catch (err) {
            console.warn("Native Print: POS receipt print failed.", err);
            return {
                reason: _t("Reason: the print job could not be queued."),
                code: "POS_ENQUEUE_FAILED",
                key: idempotencyKey,
                safeRef,
                userId,
                device: target.printer.name,
                mode: effectiveMode,
            };
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
                mode: effectiveMode,
                pages: 1,
                byte_size: payloadByteSize,
            },
            { silent: true },
        ).catch(() => {});
        watchPosJob(
            bridge, idempotencyKey, rule, payloadByteSize, safeRef,
            target.printer.name, notificationEnv(plugin), userId, skipMarkPrinted ? null : order, bridgeJobId,
        ).catch(() => {});
        if (order && !skipMarkPrinted) {
            order._printed = true;
        }
        return null;
    } finally {
        inFlightPosKeys.delete(idempotencyKey);
    }
}

patch(PosTicketPrinterPlugin.prototype, {
    async printOrderReceipt(options = {}) {
        if (!isNativeApp()) {
            return super.printOrderReceipt(...arguments);
        }
        const opts = options && typeof options === "object" ? options : {};
        const {
            basic = false,
            simplified = false,
            printBillActionTriggered = false,
        } = opts;
        const order =
            opts.order ||
            this.pos?.getOrder?.() ||
            this.pos?.get_order?.() ||
            null;
        try {
            const skipMarkPrinted = Boolean(printBillActionTriggered || basic);
            const failure = await printPosReceiptNatively(this, { order, basic, simplified, skipMarkPrinted });
            if (failure?.deduped) {
                return true;
            }
            if (!failure) {
                if (order && !printBillActionTriggered && !basic) {
                    order._printed = true;
                }
                if (!printBillActionTriggered && typeof this.markReceiptAsPrinted === "function") {
                    await this.markReceiptAsPrinted(order);
                }
                return true;
            }
            postPosDeclined(failure.key, failure.code, failure.safeRef, failure.userId, failure.device, failure.mode);
            notifyPosNativeFailure(this, failure.reason);
        } catch (error) {
            console.warn("Native Print: POS receipt print failed.", error);
            notifyPosNativeFailure(this, _t("Reason: the print job could not be queued."));
        }
        return true;
    },
});

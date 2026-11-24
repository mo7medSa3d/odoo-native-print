/** @odoo-module **/
import { PosTicketPrinterPlugin } from "@point_of_sale/app/plugins/pos_ticket_printer_plugin";
import { patch } from "@web/core/utils/patch";
import { rpc } from "@web/core/network/rpc";
import { _t } from "@web/core/l10n/translation";
import {
    JOB_TERMINAL_STATES as TERMINAL_STATES,
    notifyJobUpdated,
    notifyUser,
    recoveryButtons,
    resolveBridgeTarget,
    safeErrorCode,
} from "./native_print_client";

const POS_RECEIPT_SOURCE = "point_of_sale.receipt";
export const POS_CAPTURE_TIMEOUT_MS = 15_000;
const inFlightPosKeys = new Set();

function nativeBridge() {
    return window.__NATIVE_PRINT__ || null;
}

function isNativeApp() {
    return Boolean(nativeBridge());
}

function notificationEnv(contextObj) {
    if (contextObj?.notification && typeof contextObj.notification.add === "function") {
        return { services: { notification: contextObj.notification } };
    }
    if (contextObj?.pos?.env?.services?.notification) {
        return contextObj.pos.env;
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
    // Reuse the in-flight key within 15s for double-fire dedupe; clear on terminal state for reprints.
    const storeKey = `np.pos.key.${safeRef}.u${userId}`;
    const now = Date.now();
    let record = null;
    try {
        record = JSON.parse(sessionStorage.getItem(storeKey) || "null");
    } catch (_err) { /* storage unavailable */ }
    if (record && record.key && now - (record.ts || 0) < 15000) {
        return record.key;
    }
    const attempt = (record && record.attempt ? record.attempt : 0) + 1;
    const day = new Date().toISOString().slice(0, 10).replace(/-/g, "");
    const key = `pos-${safeRef}-${day}-u${userId}-a${attempt}`;
    try {
        sessionStorage.setItem(storeKey, JSON.stringify({ key, ts: now, attempt }));
    } catch (_err) { /* ignore */ }
    return key;
}

function finishPosReceiptAttempt(safeRef, userId) {
    // Keep the attempt counter as a tombstone so the next deliberate reprint gets a fresh key.
    try {
        const storeKey = `np.pos.key.${safeRef}.u${userId}`;
        const record = JSON.parse(sessionStorage.getItem(storeKey) || "null");
        sessionStorage.setItem(storeKey, JSON.stringify({
            key: null, ts: 0, attempt: record && record.attempt ? record.attempt : 1,
        }));
    } catch (_err) { /* ignore */ }
}

async function resolveLivePosTarget(bridge) {
    const opts = { isPos: true, preferClasses: ["thermal", "label"] };
    let target = await resolveBridgeTarget(bridge, opts).catch(() => null);
    if (!target) {
        await new Promise((resolve) => setTimeout(resolve, 700));
        target = await resolveBridgeTarget(bridge, opts).catch(() => null);
    }
    return target;
}

async function watchPosJob(bridge, jobId, rule, byteSize, safeRef, printerName, env, userId, order) {
    const absoluteDeadline = Date.now() + 30 * 60 * 1000;
    let lastProgressAt = Date.now();
    let lastSeenState = "queued";
    let lastSeenAttempts = 1;
    let reachedTerminal = false;
    let consecutiveErrors = 0;
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
            setTimeout(resolve, i === 0 ? 120 : (i < 10 ? 450 : 2000)));
        try {
            let state = null;
            let errorCode = null;
            let attempts = lastSeenAttempts;
            if (typeof bridge.jobDetail === "function") {
                const detail = await bridge.jobDetail(jobId);
                if (detail) {
                    state = detail.state;
                    errorCode = detail.errorCode || detail.error_code || null;
                    attempts = detail.attempts || attempts;
                }
            } else if (typeof bridge.printStatus === "function") {
                state = await bridge.printStatus(jobId);
            }
            consecutiveErrors = 0;
            if (!state) {
                continue;
            }
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
                const recheck = await bridge.jobDetail(jobId).catch(() => null);
                if (recheck && recheck.state && !TERMINAL_STATES.has(recheck.state)) {
                    lastSeenState = recheck.state;
                    lastSeenAttempts = recheck.attempts || attempts;
                    lastProgressAt = Date.now();
                    await syncJobStatus(
                        recheck.state,
                        recheck.errorCode || recheck.error_code || null,
                        lastSeenAttempts,
                    );
                    continue;
                }
                if (recheck && recheck.state) {
                    state = recheck.state;
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

async function renderReceiptIframe(plugin, receiptProps) {
    if (typeof plugin?.generateIframe !== "function") {
        throw new Error("generateIframe unavailable");
    }
    if (plugin.generateIframe.length === 1) {
        return plugin.generateIframe(receiptProps);
    }
    return plugin.generateIframe("point_of_sale.pos_order_receipt", receiptProps);
}

async function printPosReceiptNatively(plugin, { order, basic = false, simplified = false }) {
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
        try {
            iframe = await withTimeout(
                renderReceiptIframe(plugin, { order, basic, simplified }),
                POS_CAPTURE_TIMEOUT_MS,
                "generateIframe",
                removeIframe,
            );
            const receiptEl =
                iframe?.contentDocument?.getElementById?.("pos-receipt") ||
                iframe?.contentDocument?.body ||
                null;
            try {
                const canvas = await withTimeout(
                    plugin.generateImage(iframe),
                    POS_CAPTURE_TIMEOUT_MS,
                    "generateImage",
                );
                bytes = await withTimeout(
                    canvasToPngBytes(canvas),
                    POS_CAPTURE_TIMEOUT_MS,
                    "canvasToPngBytes",
                );
                const widthPx = receiptEl?.offsetWidth || canvas?.width || 0;
                if (widthPx) {
                    sourceWidthMm = Math.round(((widthPx * 25.4) / 96) * 10) / 10;
                }
            } catch (captureError) {
                console.warn("Native Print: Odoo 20 receipt image capture failed.", captureError);
                if (effectiveMode === "escpos" && typeof bridge.printText === "function") {
                    const extracted = extractReceiptText(receiptEl);
                    if (extracted) {
                        fallbackText = extracted;
                    }
                }
            }
        } catch (iframeError) {
            console.warn("Native Print: Odoo 20 receipt iframe generation failed.", iframeError);
        } finally {
            removeIframe(iframe);
            iframe = null;
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
        try {
            if (fallbackText) {
                await bridge.printText({
                    idempotencyKey,
                    reportName: POS_RECEIPT_SOURCE,
                    printerPattern: rule.printer_pattern,
                    text: fallbackText,
                    cut: true,
                    copies: rule.copies || 1,
                });
            } else {
                await bridge.printImage({
                    idempotencyKey,
                    reportName: POS_RECEIPT_SOURCE,
                    printerPattern: rule.printer_pattern,
                    mode: effectiveMode,
                    mediaRollWidthMm: (rule.media && rule.media.roll_width_mm) || undefined,
                    sourceWidthMm,
                    copies: rule.copies || 1,
                    bytes,
                });
            }
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
            target.printer.name, notificationEnv(plugin), userId, order,
        ).catch(() => {});
        if (order) {
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
            const failure = await printPosReceiptNatively(this, { order, basic, simplified });
            if (failure?.deduped) {
                return true;
            }
            if (!failure) {
                if (order) {
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

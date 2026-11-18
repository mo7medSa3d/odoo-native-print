/** @odoo-module **/
import { ReceiptScreen } from "@point_of_sale/app/screens/receipt_screen/receipt_screen";
import { PaymentScreen } from "@point_of_sale/app/screens/payment_screen/payment_screen";
import { TicketScreen } from "@point_of_sale/app/screens/ticket_screen/ticket_screen";
import { patch } from "@web/core/utils/patch";
import { rpc } from "@web/core/network/rpc";
import { _t } from "@web/core/l10n/translation";
import { JOB_TERMINAL_STATES as TERMINAL_STATES, notifyUser, resolveBridgeTarget } from "./native_print_client";

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
 * 2. TicketScreen can render a past receipt offscreen and call
 *    `pos.printer.printHtml(element)`. We wrap that explicit-element
 *    entry point as well, capture the supplied DOM node, and route it
 *    through the same native image queue. A missing receipt element
 *    still declines cleanly so Odoo's standard path can handle it.
 * 3. We also patch `ReceiptScreen.prototype.printReceipt` for Odoo 17.
 */

const POS_RECEIPT_SOURCE = "point_of_sale.receipt";
// TERMINAL_STATES is imported from native_print_client (the
// shared job-state contract).

function nativeBridge() {
    return window.__NATIVE_PRINT__ || null;
}

function notificationEnv(contextObj) {
    // OWL components expose env directly, PosStore keeps it on .env,
    // and some Odoo POS services attach .notification directly.
    return (
        (contextObj && contextObj.env) ||
        (contextObj && contextObj.pos && contextObj.pos.env) ||
        (contextObj && contextObj.notification
            ? { services: { notification: contextObj.notification } }
            : null) ||
        (contextObj && contextObj.pos && contextObj.pos.notification
            ? { services: { notification: contextObj.pos.notification } }
            : null) ||
        null
    );
}

function notifyPosNativeFailure(contextObj, reason) {
    // One clear message when native receipt printing gives up
    // inside the desktop app. Nothing was printed: the receipt
    // stays on screen and the cashier can fix the cause and click
    // Print again. Identical repeats are deduped inside notifyUser,
    // so the two wrapper layers below can never stack this toast.
    // The reason rides along (owner field loop, 2026-11-06): the
    // generic toast alone made every failure class — no printer,
    // capture failure, enqueue rejection, job failure — look
    // identical, including in screenshots sent for support.
    let message = _t("Native Print could not print this receipt, and nothing was printed. Check the printer and try again.");
    if (reason) {
        message += " " + reason;
    }
    notifyUser(notificationEnv(contextObj), message, "warning");
}

const POS_FAIL_NO_PRINTER = _t("Reason: no printer is available on this computer.");
const POS_FAIL_NO_RECEIPT = _t("Reason: the receipt is not on screen.");
const POS_FAIL_CAPTURE = _t("Reason: the receipt could not be captured as an image.");

const inFlightPosKeys = new Set();

function findReceiptElement(orderRef) {
    if (orderRef) {
        const shortRef = String(orderRef).replace(/^Order\s+/i, "").trim();
        const candidates = document.querySelectorAll(".pos-receipt");
        for (const el of candidates) {
            const text = el.textContent || "";
            if (text.includes(orderRef) || (shortRef && text.includes(shortRef))) {
                return el;
            }
        }
        if (candidates.length === 1) {
            return candidates[0];
        }
        if (candidates.length) {
            return null;
        }
    }
    return (
        document.querySelector(".pos-receipt-container .pos-receipt") ||
        document.querySelector(".pos-receipt") ||
        document.querySelector(".pos-receipt-container")
    );
}

async function waitForReceiptElement(orderRef, timeoutMs = 2000) {
    let el = findReceiptElement(orderRef);
    if (el) return el;
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 80));
        el = findReceiptElement(orderRef);
        if (el) return el;
    }
    return null;
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

// A 1x1 transparent PNG: the stand-in for an image that could not
// be inlined. An external URL left in the serialized receipt is
// fatal, not cosmetic — the receipt is rasterized as an SVG image,
// and SVG-as-image cannot load external resources, so one
// unfetchable logo/QR makes the whole capture throw and the
// receipt never reaches a printer (owner field report,
// 2026-11-06). Dropping the one image keeps the receipt printable.
const TRANSPARENT_PIXEL =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

async function inlineMedia(source, clone) {
    const sourceImgs = source.querySelectorAll("img");
    const cloneImgs = clone.querySelectorAll("img");
    for (let i = 0; i < sourceImgs.length && i < cloneImgs.length; i++) {
        const src = sourceImgs[i].currentSrc || sourceImgs[i].src;
        if (src && !src.startsWith("data:")) {
            const dataUrl = await fetchAsDataUrl(src);
            cloneImgs[i].setAttribute("src", dataUrl || TRANSPARENT_PIXEL);
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
            } else {
                // Same rule as <img>: an external background URL
                // left in the SVG can fail the whole capture.
                cloneAll[i].style.backgroundImage = "none";
            }
        }
    }
}

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

async function elementToPngBytes(element, scale = 2) {
    let offscreenHost = null;
    let svgObjectUrl = null;
    if (!element.isConnected && document.body) {
        offscreenHost = document.createElement("div");
        offscreenHost.className = "pos-receipt-container";
        offscreenHost.setAttribute(
            "style",
            "position:fixed;left:-99999px;top:0;width:300px;background:#fff;color:#000;pointer-events:none;z-index:-1;",
        );
        offscreenHost.appendChild(element);
        document.body.appendChild(offscreenHost);
    }
    try {
        const width = element.offsetWidth || 320;
        const height = element.offsetHeight || element.scrollHeight || 480;
        if (!width || !height) {
            throw new Error("receipt element has no size");
        }
        // Clamp scale so extra-tall receipts stay within browser canvas backing limits.
        const effectiveScale = Math.max(
            0.5,
            Math.min(scale, 14000 / height, 4096 / width),
        );
        const clone = element.cloneNode(true);
        inlineStyles(element, clone);
        await inlineMedia(element, clone);
        clone.setAttribute("xmlns", "http://www.w3.org/1999/xhtml");
        // Strip control characters that are illegal in XML 1.0 foreignObject trees.
        const serializedClone = new XMLSerializer()
            .serializeToString(clone)
            .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F]/g, "");
        const svg =
            `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">` +
            `<foreignObject width="100%" height="100%">` +
            serializedClone +
            `</foreignObject></svg>`;
        const img = new Image();
        const loaded = new Promise((resolve, reject) => {
            img.onload = resolve;
            img.onerror = () => reject(new Error("receipt SVG failed to load"));
        });
        if (typeof URL !== "undefined" && typeof URL.createObjectURL === "function") {
            const svgBlob = new Blob([svg], { type: "image/svg+xml;charset=utf-8" });
            svgObjectUrl = URL.createObjectURL(svgBlob);
            img.src = svgObjectUrl;
        } else {
            img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg);
        }
        await loaded;
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.ceil(width * effectiveScale));
        canvas.height = Math.max(1, Math.ceil(height * effectiveScale));
        const ctx = canvas.getContext("2d");
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(0, 0, canvas.width, canvas.height);
        ctx.scale(effectiveScale, effectiveScale);
        ctx.drawImage(img, 0, 0, width, height);
        const blob = await new Promise((resolve) =>
            canvas.toBlob(resolve, "image/png"),
        );
        if (!blob) {
            throw new Error("receipt canvas produced no image");
        }
        return Array.from(new Uint8Array(await blob.arrayBuffer()));
    } finally {
        if (svgObjectUrl && typeof URL !== "undefined" && typeof URL.revokeObjectURL === "function") {
            try {
                URL.revokeObjectURL(svgObjectUrl);
            } catch (_err) { /* ignore */ }
        }
        if (offscreenHost) {
            offscreenHost.remove();
        }
    }
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
    let lastSeenState = "queued";
    let lastSeenAttempts = 1;
    let reachedTerminal = false;
    let consecutiveErrors = 0;
    // The watch continues past retryable outcomes (failed /
    // dead_letter: the worker may re-queue the job), so outcome
    // handling must run once per (state, attempts) — otherwise
    // the same failure would re-post and re-notify on every poll.
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
            setTimeout(resolve, i < 10 ? 600 : 2000));
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
            // Quiet retry check: if the desktop worker hit a transient error
            // and already re-queued the receipt (`queued` / `printing`) or
            // succeeded on retry, avoid flashing a false red toast.
            if (state === "failed" && typeof bridge.jobDetail === "function") {
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
                // Same outcome as last poll: still waiting on
                // a possible retry. Do not re-post or re-notify.
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
            if (state === "failed" || state === "dead_letter") {
                notifyUser(
                    env,
                    _t('Receipt printing failed on "%s" (%s). Check the printer before reprinting.', printerName || rule.printer_pattern, errorCode || state),
                    "danger",
                );
                continue;
            }
            break;
        } catch (_err) {
            // Same rule as report jobs: one flaky poll must not
            // orphan the job and hide its outcome from the cashier.
            if (++consecutiveErrors >= 10) {
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

// A declined POS attempt (no printer, capture failure, enqueue
// rejection) creates no desktop job, so without this post it
// leaves NO trace anywhere — not in the Control Center, not in
// the server log — and every failure looked identical (owner
// field loop, 2026-11-06). Post it to the job log with its reason
// code, exactly like a real job's failure, so the cause is
// readable in Odoo's Print Jobs afterwards.
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
    rpc("/native_print/job_status", params, { silent: true }).catch(() => {});
    // The attempt is over: rotate the key so the cashier's next
    // click is a fresh attempt (the posted row is terminal).
    if (safeRef) { finishPosReceiptAttempt(safeRef, userId); }
}

async function printPosReceiptNatively(contextObj, explicitOrder = null, explicitElement = null) {
    // Returns null on success, or {reason, code, key, safeRef,
    // userId, device, mode} describing the decline for the wrapper
    // to log and show. Enqueue rejections throw with npKey/npRef
    // attached (the wrapper logs and reports them).
    const bridge = nativeBridge();
    const order =
        explicitOrder ||
        (contextObj && contextObj.currentOrder) ||
        (contextObj && contextObj.getOrder ? contextObj.getOrder() : null) ||
        (contextObj && contextObj.get_order ? contextObj.get_order() : null) ||
        (contextObj && contextObj.pos && contextObj.pos.getOrder
            ? contextObj.pos.getOrder()
            : null) ||
        (contextObj && contextObj.pos && contextObj.pos.get_order
            ? contextObj.pos.get_order()
            : null);
    const displayRef =
        (order && (order.name || order.pos_reference || order.uid || order.uuid)) ||
        null;
    const keyRef =
        (order && (order.uuid || order.uid || order.name || order.pos_reference)) ||
        "pos-order";
    const safeRef = String(keyRef).replace(/[^A-Za-z0-9_-]/g, "_").slice(0, 40);
    // The cashier printing right now (audit H1): part of the job key
    // and of the in-flight record, because the server log scopes jobs
    // per user and one browser profile can serve several cashiers.
    const posStore = (contextObj && contextObj.pos) || contextObj || null;
    const cashierObj =
        (posStore && typeof posStore.get_cashier === "function" && posStore.get_cashier()) ||
        (posStore && posStore.cashier) ||
        null;
    const cashierRaw =
        (cashierObj && (cashierObj.id || (Array.isArray(cashierObj) && cashierObj[0]) || cashierObj)) ||
        (posStore && posStore.user && (posStore.user.id || (Array.isArray(posStore.user) && posStore.user[0]) || posStore.user)) ||
        (posStore && posStore.session && (posStore.session.user_id?.id || (Array.isArray(posStore.session.user_id) && posStore.session.user_id[0]) || posStore.session.user_id)) ||
        (order && order.user_id && (order.user_id.id || (Array.isArray(order.user_id) && order.user_id[0]) || order.user_id)) ||
        0;
    const userId = String(cashierRaw).replace(/[^0-9]/g, "") || "0";
    const idempotencyKey = posReceiptKey(safeRef, userId);
    if (!bridge || typeof bridge.printImage !== "function") {
        return { reason: POS_FAIL_NO_PRINTER, code: "POS_NO_PRINTER", key: idempotencyKey, safeRef, userId };
    }
    if (inFlightPosKeys.has(idempotencyKey)) {
        return null;
    }
    inFlightPosKeys.add(idempotencyKey);
    try {
        let target = await resolveBridgeTarget(
            bridge, { isPos: true, preferClasses: ["thermal", "label"] },
        ).catch(() => null);
        if (!target) {
            await new Promise((resolve) => setTimeout(resolve, 700));
            target = await resolveBridgeTarget(
                bridge, { isPos: true, preferClasses: ["thermal", "label"] },
            ).catch(() => null);
        }
        if (!target) {
            // No printer on this PC right now.
            return { reason: POS_FAIL_NO_PRINTER, code: "POS_NO_PRINTER", key: idempotencyKey, safeRef, userId };
        }
        const element = explicitElement || (await waitForReceiptElement(order ? displayRef : null));
        if (!element) {
            return { reason: POS_FAIL_NO_RECEIPT, code: "POS_RECEIPT_NOT_ON_SCREEN", key: idempotencyKey, safeRef, userId, device: target.printer.name, mode: target.rule.mode };
        }
        const rule = target.rule;
        // The mode comes from the TARGET printer's detection (thermal
        // -> escpos, label languages, office/virtual -> gdi).
        const effectiveMode = rule.mode || "gdi";
        const sourceWidthMm = element.offsetWidth
            ? Math.round((element.offsetWidth * 25.4) / 96 * 10) / 10
            : undefined;
        let bytes = null;
        let fallbackText = null;
        try {
            bytes = await elementToPngBytes(element, 2);
        } catch (_firstCaptureError) {
            // A capture can fail on canvas size alone (a very long
            // receipt at scale 2); retry once at scale 1, and if raster
            // capture still fails on a thermal ESC/POS printer, fall back
            // to plain-text thermal output instead of aborting the receipt.
            try {
                bytes = await elementToPngBytes(element, 1);
            } catch (_secondCaptureError) {
                if (effectiveMode === "escpos" && typeof bridge.printText === "function") {
                    const extracted = extractReceiptText(element);
                    if (extracted) {
                        fallbackText = extracted;
                    }
                }
                if (!fallbackText) {
                    return { reason: POS_FAIL_CAPTURE, code: "POS_CAPTURE_FAILED", key: idempotencyKey, safeRef, userId, device: target.printer.name, mode: effectiveMode };
                }
            }
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
                    mediaRollWidthMm:
                        (rule.media && rule.media.roll_width_mm) || undefined,
                    sourceWidthMm,
                    copies: rule.copies || 1,
                    bytes,
                });
            }
        } catch (err) {
            if (safeRef) { finishPosReceiptAttempt(safeRef, userId); }
            try {
                err.npKey = idempotencyKey;
                err.npRef = safeRef;
                err.npUserId = userId;
                err.npDevice = target.printer.name;
                err.npMode = effectiveMode;
            } catch (_ignored) { /* non-object error: nothing to attach */ }
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
                mode: effectiveMode,
                pages: 1,
                byte_size: payloadByteSize,
            },
            { silent: true },
        ).catch(() => {});
        watchPosJob(
            bridge, idempotencyKey, rule, payloadByteSize, safeRef,
            target.printer.name, notificationEnv(contextObj), userId, order,
        ).catch(() => {});
        if (order) {
            order._printed = true;
        }
        return null;
    } finally {
        inFlightPosKeys.delete(idempotencyKey);
    }
}

function reportPosFailure(contextObj, failure) {
    postPosDeclined(failure.key, failure.code, failure.safeRef, failure.userId, failure.device, failure.mode);
    notifyPosNativeFailure(contextObj, failure.reason);
}

function reportPosException(contextObj, error, label) {
    console.warn(`Native Print: POS ${label} capture failed.`, error);
    if (error && error.npKey) {
        postPosDeclined(error.npKey, "POS_ENQUEUE_FAILED", error.npRef || null, error.npUserId || null, error.npDevice, error.npMode);
    }
    notifyPosNativeFailure(
        contextObj,
        _t("Reason: the print job could not be queued.") +
            " " + String((error && error.message) || error || ""),
    );
}

function wrapPosPrinterService(posStore) {
    const printer = posStore && posStore.printer;
    if (!printer || printer.__nativePrintHtmlWrapped || typeof printer.printHtml !== "function") {
        return;
    }
    const originalPrintHtml = printer.printHtml.bind(printer);
    printer.printHtml = async function (el, options = {}) {
        if (!nativeBridge()) {
            return await originalPrintHtml(el, options);
        }
        try {
            const explicitOrder = posStore.__pendingNativePrintOrder || null;
            const failure = await printPosReceiptNatively(posStore, explicitOrder, el);
            if (!failure) {
                return true;
            }
            reportPosFailure(posStore, failure);
        } catch (error) {
            reportPosException(posStore, error, "printHtml");
        }
        return true;
    };
    printer.__nativePrintHtmlWrapped = true;
}

function wrapPosStorePrintReceipt(posStore) {
    if (!posStore) {
        return;
    }
    wrapPosPrinterService(posStore);
    if (posStore.__nativePrintWrapped || typeof posStore.printReceipt !== "function") {
        return;
    }
    const originalPrintReceipt = posStore.printReceipt.bind(posStore);
    posStore.printReceipt = async function (opts = {}) {
        if (!nativeBridge()) {
            // Plain browser: Odoo's standard printing is the only
            // printing there is (owner's standing rule for browsers).
            return await originalPrintReceipt(opts);
        }
        wrapPosPrinterService(posStore);
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
            const failure = await printPosReceiptNatively(posStore, opts && opts.order);
            if (!failure) {
                return true;
            }
            if (
                failure.code === "POS_RECEIPT_NOT_ON_SCREEN" &&
                posStore.printer &&
                posStore.printer.__nativePrintHtmlWrapped
            ) {
                posStore.__pendingNativePrintOrder = (opts && opts.order) || null;
                try {
                    return await originalPrintReceipt(opts);
                } finally {
                    posStore.__pendingNativePrintOrder = null;
                }
            }
            reportPosFailure(posStore, failure);
        } catch (error) {
            reportPosException(posStore, error, "receipt");
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
        // Route through the wrapped PosStore.printReceipt when present so
        // manual/auto/TicketScreen prints share one handler and fallback path.
        if (this.pos && typeof this.pos.printReceipt === "function") {
            wrapPosStorePrintReceipt(this.pos);
            return await this.pos.printReceipt(...arguments);
        }
        try {
            const failure = await printPosReceiptNatively(this, opts && opts.order);
            if (!failure) {
                return true;
            }
            reportPosFailure(this, failure);
        } catch (error) {
            reportPosException(this, error, "receipt");
        }
        return true;
    },
});

patch(PaymentScreen.prototype, {
    setup() {
        super.setup(...arguments);
        if (this.pos) {
            wrapPosStorePrintReceipt(this.pos);
        }
    },
});

patch(TicketScreen.prototype, {
    setup() {
        super.setup(...arguments);
        if (this.pos) {
            wrapPosStorePrintReceipt(this.pos);
        }
    },
});

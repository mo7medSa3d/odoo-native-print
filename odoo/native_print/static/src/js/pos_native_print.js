/** @odoo-module **/
import { ReceiptScreen } from "@point_of_sale/app/screens/receipt_screen/receipt_screen";
import { PaymentScreen } from "@point_of_sale/app/screens/payment_screen/payment_screen";
import { TicketScreen } from "@point_of_sale/app/screens/ticket_screen/ticket_screen";
import { patch } from "@web/core/utils/patch";
import { rpc } from "@web/core/network/rpc";
import { _t } from "@web/core/l10n/translation";
import { JOB_TERMINAL_STATES as TERMINAL_STATES, notifyJobUpdated, notifyUser, resolveBridgeTarget } from "./native_print_client";

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
    // Append the failure reason so distinct failure classes are actionable.
    let message = _t("Native Print could not print this receipt, and nothing was printed. Check the printer and try again.");
    if (reason) {
        message += " " + reason;
    }
    notifyUser(notificationEnv(contextObj), message, "warning");
}

const posFailNoPrinter = () => _t("Reason: no printer is available on this computer.");
const posFailNoReceipt = () => _t("Reason: the receipt is not on screen.");
const posFailCapture = () => _t("Reason: the receipt could not be captured as an image.");

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

const MEDIA_FETCH_TIMEOUT_MS = 8000;

function inlineStyles(source, clone, bgTargets = []) {
    const computed = window.getComputedStyle(source);
    let css = computed.cssText;
    if (!css) {
        css = "";
        for (let i = 0; i < computed.length; i++) {
            const prop = computed[i];
            css += `${prop}:${computed.getPropertyValue(prop)};`;
        }
    }
    // Strip external url(...) references from serialized CSS so no uninlined
    // resource (mask-image, cursor, list-style-image, multi-bg) can fail SVG
    // foreignObject loading or taint the canvas in Chromium/WebView2.
    css = css.replace(/url\(\s*(?:"(?!data:)[^"]*"|'(?!data:)[^']*'|(?![\s"']*data:)[^)]+)\s*\)/gi, "none");
    clone.setAttribute("style", css);
    // Collect external CSS background images during the same walk (including
    // the root receipt node) to avoid a second getComputedStyle DOM pass.
    const bg = computed.backgroundImage;
    const match = bg && bg.match(/url\(["']?(.*?)["']?\)/);
    if (match && match[1] && !match[1].startsWith("data:")) {
        bgTargets.push({ node: clone, url: match[1] });
    }
    const sourceKids = source.children;
    const cloneKids = clone.children;
    for (let i = 0; i < sourceKids.length && i < cloneKids.length; i++) {
        inlineStyles(sourceKids[i], cloneKids[i], bgTargets);
    }
    return bgTargets;
}

async function fetchAsDataUrl(url, timeoutMs = MEDIA_FETCH_TIMEOUT_MS) {
    const controller = typeof AbortController === "function" ? new AbortController() : null;
    const timer = controller ? setTimeout(() => controller.abort(), timeoutMs) : null;
    try {
        const opts = controller
            ? { credentials: "same-origin", signal: controller.signal }
            : { credentials: "same-origin" };
        const resp = await fetch(url, opts);
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
    } finally {
        if (timer !== null) {
            clearTimeout(timer);
        }
    }
}

// 1x1 transparent PNG stand-in for unfetchable images so an external URL
// never causes SVG foreignObject rasterization to fail the whole receipt.
const TRANSPARENT_PIXEL =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

async function inlineMedia(source, clone, bgTargets = []) {
    const urlCache = new Map();
    const cachedFetch = (url) => {
        if (!urlCache.has(url)) {
            urlCache.set(url, fetchAsDataUrl(url));
        }
        return urlCache.get(url);
    };
    const imgToDataUrlFallback = (imgEl) => {
        try {
            if (!imgEl || !imgEl.complete || !imgEl.naturalWidth || !imgEl.naturalHeight) {
                return null;
            }
            if (typeof document === "undefined" || typeof document.createElement !== "function") {
                return null;
            }
            const c = document.createElement("canvas");
            c.width = imgEl.naturalWidth;
            c.height = imgEl.naturalHeight;
            const cx = c.getContext && c.getContext("2d");
            if (!cx) return null;
            cx.drawImage(imgEl, 0, 0);
            return c.toDataURL("image/png");
        } catch (_err) {
            return null;
        }
    };
    const sourceImgs = source.querySelectorAll("img");
    const cloneImgs = clone.querySelectorAll("img");
    for (let i = 0; i < sourceImgs.length && i < cloneImgs.length; i++) {
        if (typeof cloneImgs[i].removeAttribute === "function") {
            cloneImgs[i].removeAttribute("srcset");
            cloneImgs[i].removeAttribute("loading");
            cloneImgs[i].removeAttribute("decoding");
        }
        const src = sourceImgs[i].currentSrc || sourceImgs[i].src;
        if (src && !src.startsWith("data:")) {
            const dataUrl = (await cachedFetch(src)) || imgToDataUrlFallback(sourceImgs[i]);
            cloneImgs[i].setAttribute("src", dataUrl || TRANSPARENT_PIXEL);
        }
    }
    const sourceSvgImgs = source.querySelectorAll("image");
    const cloneSvgImgs = clone.querySelectorAll("image");
    for (let i = 0; i < sourceSvgImgs.length && i < cloneSvgImgs.length; i++) {
        const href =
            (typeof sourceSvgImgs[i].getAttribute === "function" &&
                (sourceSvgImgs[i].getAttribute("href") ||
                    sourceSvgImgs[i].getAttribute("xlink:href"))) ||
            "";
        if (href && !href.startsWith("data:")) {
            const dataUrl = (await cachedFetch(href)) || TRANSPARENT_PIXEL;
            cloneSvgImgs[i].setAttribute("href", dataUrl);
            if (typeof cloneSvgImgs[i].removeAttribute === "function") {
                cloneSvgImgs[i].removeAttribute("xlink:href");
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
            const canvasStyle = cloneCanvases[i].getAttribute("style");
            if (canvasStyle) {
                img.setAttribute("style", canvasStyle);
            }
            cloneCanvases[i].replaceWith(img);
        } catch (_err) {
            // Leave cross-origin canvas as-is.
        }
    }
    // Inline CSS background images collected during inlineStyles().
    for (const { node, url } of bgTargets) {
        const dataUrl = await cachedFetch(url);
        if (dataUrl) {
            node.style.backgroundImage = `url("${dataUrl}")`;
        } else {
            // Same rule as <img>: an external background URL
            // left in the SVG can fail the whole capture.
            node.style.backgroundImage = "none";
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

async function canvasToPngBytes(canvas) {
    if (typeof canvas.toBlob === "function") {
        const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
        if (blob) {
            return Array.from(new Uint8Array(await blob.arrayBuffer()));
        }
    }
    if (typeof canvas.toDataURL === "function") {
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

async function withConnectedElement(element, fn) {
    let offscreenHost = null;
    let prevParent = null;
    let prevNext = null;
    if (!element.isConnected && document.body) {
        prevParent = element.parentNode || null;
        prevNext = element.nextSibling || null;
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
        return await fn();
    } finally {
        if (offscreenHost) {
            if (prevParent && typeof prevParent.insertBefore === "function") {
                prevParent.insertBefore(element, prevNext);
            } else if (typeof element.remove === "function") {
                element.remove();
            }
            offscreenHost.remove();
        }
    }
}

async function elementToPngBytes(element, scale = 2) {
    return await withConnectedElement(element, async () => {
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
        const bgTargets = inlineStyles(element, clone);
        await inlineMedia(element, clone, bgTargets);
        // Reset root positioning/overflow so the receipt fills the foreignObject
        // cleanly, and preserve direction/lang for RTL Arabic rendering.
        const rootStyle = (clone.getAttribute("style") || "") +
            ";margin:0 !important;position:static !important;transform:none !important;max-height:none !important;overflow:visible !important;box-sizing:border-box !important;";
        clone.setAttribute("style", rootStyle);
        const rootDir = element.getAttribute("dir") ||
            (window.getComputedStyle && window.getComputedStyle(element).direction) ||
            (document.documentElement && document.documentElement.dir) || "";
        if (rootDir) clone.setAttribute("dir", rootDir);
        const rootLang = element.getAttribute("lang") ||
            (document.documentElement && document.documentElement.lang) || "";
        if (rootLang) clone.setAttribute("lang", rootLang);
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
        let svgTimer = null;
        const loaded = new Promise((resolve, reject) => {
            svgTimer = setTimeout(
                () => reject(new Error("receipt SVG load timed out")),
                MEDIA_FETCH_TIMEOUT_MS,
            );
            img.onload = () => {
                clearTimeout(svgTimer);
                resolve();
            };
            img.onerror = () => {
                clearTimeout(svgTimer);
                reject(new Error("receipt SVG failed to load"));
            };
        });
        // Always use a data: URI (never URL.createObjectURL): in Chromium /
        // Edge WebView2, a blob: SVG with <foreignObject> taints the target
        // canvas and makes canvas.toBlob() throw SecurityError / return null.
        img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg);
        await loaded;
        if (typeof img.decode === "function") {
            await img.decode().catch(() => {});
        }
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.ceil(width * effectiveScale));
        canvas.height = Math.max(1, Math.ceil(height * effectiveScale));
        const ctx = canvas.getContext("2d");
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(0, 0, canvas.width, canvas.height);
        ctx.scale(effectiveScale, effectiveScale);
        ctx.drawImage(img, 0, 0, width, height);
        return await canvasToPngBytes(canvas);
    });
}

// Pure 2D canvas fallback when SVG foreignObject capture is blocked by a
// WebView/CSP quirk: draws images/QR codes, horizontal rules, and positioned
// text runs directly without SVG so receipts still print on every printer.
async function renderReceiptCanvasFallback(element, scale = 2) {
    return await withConnectedElement(element, async () => {
        const rootRect = element.getBoundingClientRect();
        const width = Math.ceil(element.offsetWidth || rootRect.width || 320);
        const height = Math.ceil(element.offsetHeight || element.scrollHeight || rootRect.height || 480);
        if (!width || !height) {
            throw new Error("receipt element has no size");
        }
        const effectiveScale = Math.max(0.5, Math.min(scale, 14000 / height, 4096 / width));
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.ceil(width * effectiveScale));
        canvas.height = Math.max(1, Math.ceil(height * effectiveScale));
        const ctx = canvas.getContext("2d");
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(0, 0, canvas.width, canvas.height);
        ctx.scale(effectiveScale, effectiveScale);

        // Draw visible images (logo, QR code, barcodes) and canvases at their layout boxes.
        for (const media of element.querySelectorAll("img, canvas")) {
            const r = media.getBoundingClientRect();
            if (!r.width || !r.height) continue;
            if (media.tagName === "IMG" && (!media.complete || !media.naturalWidth)) continue;
            try {
                ctx.drawImage(media, r.left - rootRect.left, r.top - rootRect.top, r.width, r.height);
            } catch (_err) { /* skip cross-origin image without tainting text */ }
        }

        // Draw horizontal borders / dividers.
        for (const el of element.querySelectorAll("*")) {
            const st = window.getComputedStyle(el);
            const r = el.getBoundingClientRect();
            if (!r.width || !r.height) continue;
            const bt = parseFloat(st.borderTopWidth) || 0;
            if (bt > 0 && st.borderTopStyle !== "none") {
                ctx.fillStyle = st.borderTopColor || "#000000";
                ctx.fillRect(r.left - rootRect.left, r.top - rootRect.top, r.width, bt);
            }
            const bb = parseFloat(st.borderBottomWidth) || 0;
            if (bb > 0 && st.borderBottomStyle !== "none") {
                ctx.fillStyle = st.borderBottomColor || "#000000";
                ctx.fillRect(r.left - rootRect.left, r.bottom - rootRect.top - bb, r.width, bb);
            }
        }

        // Draw visible text nodes at their exact layout rects.
        if (typeof document.createTreeWalker === "function" && typeof document.createRange === "function") {
            const walker = document.createTreeWalker(element, 4 /* NodeFilter.SHOW_TEXT */);
            const range = document.createRange();
            let node;
            while ((node = walker.nextNode())) {
                const text = (node.nodeValue || "").replace(/\s+/g, " ").trim();
                if (!text) continue;
                const parent = node.parentElement;
                if (!parent) continue;
                const st = window.getComputedStyle(parent);
                if (st.visibility === "hidden" || st.display === "none") continue;
                range.selectNodeContents(node);
                const r = range.getBoundingClientRect();
                if (!r.width || !r.height) continue;
                const fontSize = st.fontSize || "13px";
                const fontWeight = st.fontWeight || "400";
                const fontFamily = st.fontFamily || "sans-serif";
                ctx.font = `${fontWeight} ${fontSize} ${fontFamily}`;
                ctx.fillStyle = st.color || "#000000";
                ctx.textBaseline = "top";
                const dir = st.direction || "ltr";
                if ("direction" in ctx) ctx.direction = dir;
                const x = dir === "rtl" ? (r.right - rootRect.left) : (r.left - rootRect.left);
                ctx.textAlign = dir === "rtl" ? "right" : "left";
                ctx.fillText(text, x, r.top - rootRect.top, Math.max(r.width + 4, 20));
            }
        }
        return await canvasToPngBytes(canvas);
    });
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
    // Idempotency with intent: resubmitting the SAME order within a
    // few seconds is the same print (double-fire, POS retry) and must
    // dedupe on the desktop, so it reuses the in-flight key. Once the
    // job reaches a terminal state the watcher clears the record, so
    // a later deliberate reprint becomes a fresh attempt (a2, a3...).
    // The day stamp bounds key reuse to one business day.
    // Scope storage and job keys per cashier user id to prevent
    // cross-cashier collisions on the per-user server job log.
    const storeKey = `np.pos.key.${safeRef}.u${userId}`;
    const now = Date.now();
    let record = null;
    try {
        record = JSON.parse(sessionStorage.getItem(storeKey) || "null");
    } catch (_err) { /* storage unavailable: fall through */ }
    if (record && record.key && now - (record.ts || 0) < 15000) {
        // Keep the 15s dedupe window anchored to the first submission.
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
            // Quiet retry check: if the desktop worker hit a transient error
            // and already re-queued the receipt (`queued` / `printing`) or
            // succeeded on retry, avoid flashing a false red toast.
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
            notifyJobUpdated();
            if (state === "failed" || state === "dead_letter") {
                notifyUser(
                    env,
                    _t('Receipt printing failed on "%s" (%s). Check the printer before reprinting.', printerName || rule.printer_pattern, errorCode || state),
                    "danger",
                );
                if (state === "dead_letter") {
                    break;
                }
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

// Log declined POS attempts (no printer, capture failure, enqueue rejection)
// with their reason code so failures remain visible in Print Jobs.
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
        `pos-html-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    const safeRef = String(keyRef).replace(/[^A-Za-z0-9_-]/g, "_").slice(0, 40);
    // Include cashier user id in the job key because the server log scopes jobs per user.
    const posStore = (contextObj && contextObj.pos) || contextObj || null;
    const cashierObj =
        (posStore && typeof posStore.get_cashier === "function" && posStore.get_cashier()) ||
        (posStore && posStore.cashier) ||
        null;
    const userId = extractCashierUserId(cashierObj, posStore, order);
    const idempotencyKey = posReceiptKey(safeRef, userId);
    if (!bridge || typeof bridge.printImage !== "function") {
        return { reason: posFailNoPrinter(), code: "POS_NO_PRINTER", key: idempotencyKey, safeRef, userId };
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
            return { reason: posFailNoPrinter(), code: "POS_NO_PRINTER", key: idempotencyKey, safeRef, userId };
        }
        const element = explicitElement || (await waitForReceiptElement(order ? displayRef : null));
        if (!element) {
            return { reason: posFailNoReceipt(), code: "POS_RECEIPT_NOT_ON_SCREEN", key: idempotencyKey, safeRef, userId, device: target.printer.name, mode: target.rule.mode };
        }
        const rule = target.rule;
        // The mode comes from the TARGET printer's detection (thermal
        // -> escpos, label languages, office/virtual -> gdi).
        const effectiveMode = rule.mode || "gdi";
        const widthPx = element.offsetWidth || (!element.isConnected ? 300 : 0);
        const sourceWidthMm = widthPx
            ? Math.round((widthPx * 25.4) / 96 * 10) / 10
            : undefined;
        let bytes = null;
        let fallbackText = null;
        try {
            bytes = await elementToPngBytes(element, 2);
        } catch (firstCaptureError) {
            console.warn("Native Print: POS SVG capture (scale 2) failed, retrying at scale 1.", firstCaptureError);
            try {
                bytes = await elementToPngBytes(element, 1);
            } catch (secondCaptureError) {
                console.warn("Native Print: POS SVG capture (scale 1) failed, trying 2D canvas fallback.", secondCaptureError);
                try {
                    bytes = await renderReceiptCanvasFallback(element, 2);
                } catch (canvasFallbackError) {
                    console.warn("Native Print: POS 2D canvas fallback failed.", canvasFallbackError);
                    if (effectiveMode === "escpos" && typeof bridge.printText === "function") {
                        const extracted = extractReceiptText(element);
                        if (extracted) {
                            fallbackText = extracted;
                        }
                    }
                    if (!fallbackText) {
                        return { reason: posFailCapture(), code: "POS_CAPTURE_FAILED", key: idempotencyKey, safeRef, userId, device: target.printer.name, mode: effectiveMode };
                    }
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
        _t("Reason: the print job could not be queued."),
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
            // Plain browser: keep Odoo's standard print flow untouched.
            return await originalPrintReceipt(opts);
        }
        wrapPosPrinterService(posStore);
        // Inside the desktop app, never fall back to the browser print dialog;
        // report the reason and leave the receipt on screen for retry.
        try {
            // Skip the 2s DOM poll on offscreen reprints (e.g. TicketScreen) when
            // no .pos-receipt is currently mounted and printHtml is already wrapped.
            if (
                opts &&
                opts.order &&
                !findReceiptElement() &&
                posStore.printer &&
                posStore.printer.__nativePrintHtmlWrapped
            ) {
                posStore.__pendingNativePrintOrder = opts.order;
                try {
                    return await originalPrintReceipt(opts);
                } finally {
                    posStore.__pendingNativePrintOrder = null;
                }
            }
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

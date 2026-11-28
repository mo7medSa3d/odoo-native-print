/** @odoo-module **/
import { registry } from "@web/core/registry";
import { rpc } from "@web/core/network/rpc";
import { user } from "@web/core/user";
import { _t } from "@web/core/l10n/translation";
import { extractLicenseErrorCode, JOB_TERMINAL_STATES as TERMINAL_STATES, licenseErrorMessage, normalizeBridgeJobState, notifyJobUpdated, notifyUser, recoveryButtons, resolveBridgeJobId, resolveBridgeTarget, safeErrorCode } from "./native_print_client";

/*
 * Itbaa report handler (API contract v1, docs/api-contract.md).
 *
 * - Normal browser: no bridge exists -> return undefined (falsy) and Odoo
 *   behaves exactly as usual. This is the most important line in the file.
 * - Inside the Itbaa app the shell injects window.__NATIVE_PRINT__
 *   on the configured origin only. Printers live on THIS PC only:
 *   the target is resolved live from the bridge inventory (the printer
 *   the user selected in the systray, else the OS default), its real
 *   capabilities are read from the driver at click time, the PDF is
 *   fetched with the user's own session, handed to the bridge, and the
 *   job outcome is reported to the server job log. Nothing about
 *   printers is stored on the server.
 * - If this PC has no printer right now, we also return undefined and
 *   Odoo's standard download happens.
 */

// TERMINAL_STATES is imported from native_print_client (the
// shared job-state contract). `failed` and `dead_letter` are
// retryable outcomes (the desktop worker re-queues a failed job
// and the operator may retry a dead-letter job): the watcher
// keeps polling after them so the retry's outcome, not the
// first failure, is what the user and the server log end with.

// The desktop queue's hard payload ceiling (print-core MAX_JOB_BYTES,
// 50 MB). Payloads above it can never be accepted, so the handler
// checks before enqueueing and keeps Odoo's standard download.
const MAX_NATIVE_PAYLOAD_BYTES = 50 * 1024 * 1024;
// Matches print_text's 1 MiB ceiling in app/src-tauri/src/commands.rs.
const MAX_NATIVE_TEXT_BYTES = 1024 * 1024;

function buildReportIdempotencyKey(prefix, action) {
    const raw = (action && (action.id || action.report_name)) || "report";
    const safeTag = String(raw).replace(/[^A-Za-z0-9._-]/g, "_").slice(0, 64) || "report";
    return `${prefix}-${safeTag}-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

// Upper safety timeout for fetching a rendered report from Odoo:
// generous enough for slow QWeb-PDF renders, bounded so a hung
// network response cannot lock the report action forever.
export const REPORT_FETCH_TIMEOUT_MS = 120_000;
export const IN_FLIGHT_STALE_MS = REPORT_FETCH_TIMEOUT_MS + 10_000;

async function fetchReportBody(url, readBody, timeoutMs = REPORT_FETCH_TIMEOUT_MS) {
    const controller = typeof AbortController === "function" ? new AbortController() : null;
    const timer = controller ? setTimeout(() => controller.abort(), timeoutMs) : null;
    try {
        let fetchUrl = url;
        const opts = controller
            ? { credentials: "same-origin", signal: controller.signal }
            : { credentials: "same-origin" };
        // Avoid HTTP 414 URI Too Long on large batch/wizard reports by using Odoo's POST /report/download.
        if (url.length > 2000 && typeof FormData === "function") {
            const reportType = url.startsWith("/report/text/") ? "qweb-text" : "qweb-pdf";
            const form = new FormData();
            form.append("data", JSON.stringify([url, reportType]));
            if (typeof window !== "undefined" && window.odoo && window.odoo.csrf_token) {
                form.append("csrf_token", window.odoo.csrf_token);
            }
            fetchUrl = "/report/download";
            opts.method = "POST";
            opts.body = form;
        }
        const resp = await fetch(fetchUrl, opts);
        if (!resp.ok) {
            return null;
        }
        // Expired session redirects to /web/login or serves the login HTML form;
        // return null so Odoo's own flow handles re-authentication instead of
        // treating the login page as a missing-PDF-engine report.
        if (resp.redirected && /\/web\/login\b/.test(String(resp.url || ""))) {
            return null;
        }
        const body = await readBody(resp);
        const contentType = String(resp.headers?.get?.("content-type") || "").toLowerCase();
        if (!contentType || contentType.includes("text/html") || contentType.includes("application/xhtml")) {
            const probe =
                typeof body === "string"
                    ? body
                    : body instanceof ArrayBuffer
                        ? new TextDecoder().decode(new Uint8Array(body, 0, Math.min(body.byteLength, 8192)))
                        : "";
            if (probe.includes("oe_login_form")) {
                return null;
            }
        }
        return body;
    } finally {
        if (timer !== null) {
            clearTimeout(timer);
        }
    }
}

// Report submissions currently between click and desktop enqueue,
// keyed by report + target records -> { startedAt }. A second
// invocation while fresh is a duplicate of the SAME user intent;
// an expired entry cannot lock the action forever.
const inFlightReports = new Map();

export function nativePrintBridge() {
    return window.__NATIVE_PRINT__ || null;
}

export function buildReportUrl(action, converter = "pdf", env = null) {
    // qweb-html actions can arrive with only report_url (the HTML client
    // action); recover the technical name from it instead of building
    // a /report/text/undefined URL.
    const fromUrl =
        String((action && action.report_url) || "").match(/\/report\/(?:html|pdf|text)\/([^/?]+)/) ||
        [];
    const reportName = (action && action.report_name) || fromUrl[1] || "report";
    let url = `/report/${converter}/${reportName}`;
    const actionContext = action.context || {};
    const userContext =
        (env && env.services && env.services.user && env.services.user.context) ||
        (user && user.context) ||
        {};
    const mergedContext = Object.assign({}, userContext, actionContext);
    const hasWizardData =
        action.data &&
        typeof action.data === "object" &&
        Object.keys(action.data).length > 0;
    if (hasWizardData) {
        const encodedOptions = encodeURIComponent(JSON.stringify(action.data));
        const encodedContext = encodeURIComponent(JSON.stringify(mergedContext));
        url += `?options=${encodedOptions}&context=${encodedContext}`;
    } else {
        const ids = pickIds(action);
        const docids = Array.isArray(ids) ? ids.join(",") : String(ids || "");
        if (docids) {
            url += `/${docids}`;
        }
        if (Object.keys(mergedContext).length > 0) {
            const encodedContext = encodeURIComponent(JSON.stringify(mergedContext));
            url += `?context=${encodedContext}`;
        }
    }
    return url;
}

function pickIds(action) {
    if (!action) return [];
    const ctx = action.context || {};
    const nonEmpty = (v) => (Array.isArray(v) ? (v.length > 0 ? v : null) : (v || null));
    return (
        nonEmpty(ctx.active_ids) ||
        (ctx.active_id ? [ctx.active_id] : null) ||
        nonEmpty(action.ids) ||
        (action.res_id ? [action.res_id] : [])
    );
}

// The source reference (model + record ids) a report is printed
// for, posted with the job so the job log can offer a real Reprint.
export function reportSource(action) {
    if (!action) {
        return { model: null, ids: "" };
    }
    const ctx = action.context || {};
    const ids = pickIds(action);
    const list = Array.isArray(ids) ? ids : [ids];
    return {
        model: ctx.active_model || action.model || null,
        ids: list.filter((x) => Number.isInteger(x) || /^\d+$/.test(String(x))).join(","),
    };
}

export function reportDataOf(action) {
    if (!action || !action.data || typeof action.data !== "object" || Array.isArray(action.data)) {
        return undefined;
    }
    return Object.keys(action.data).length > 0 ? action.data : undefined;
}

/*
 * Render a plain-text report as a PNG document (A4 width at
 * 96 dpi, monospace, hard-wrapped) so it can print through GDI
 * on office, label and virtual printers, which cannot interpret
 * ESC/POS. The desktop canvas path paginates a tall
 * image across sheets, so one long PNG is the whole document.
 * Throws when the canvas is unavailable; the caller falls back
 * to Odoo's standard download.
 */
// Odoo's "text" report converter still renders the QWeb template, so the
// response is usually HTML, not plain text. Turn it into printable lines
// without a PDF engine: block boundaries and <br> become newlines, tags
// are dropped, and the result is normalised for the canvas renderer.
export function reportHtmlToPlainText(markup) {
    const raw = String(markup || "");
    const normalise = (text) =>
        text
            .replace(/\r\n?/g, "\n")
            .replace(/[ \t\u00a0]+/g, " ")
            .split("\n")
            .map((line) => line.trim())
            .join("\n")
            .replace(/\n{3,}/g, "\n\n")
            .trim();
    if (!/<\s*[a-z!/]/i.test(raw)) {
        return normalise(raw);
    }
    // Minimal-DOM environments (and very restricted webviews) get a
    // regex conversion; real browsers use the parsed DOM for correct
    // block boundaries and entity decoding.
    const stripWithRegex = () =>
        raw
            .replace(/<\s*br\s*\/?\s*>/gi, "\n")
            .replace(/<\/(?:p|div|tr|li|h[1-5]|table|section)\s*>/gi, "\n")
            .replace(/<[^>]+>/g, " ")
            .replace(/&nbsp;/gi, " ")
            .replace(/&amp;/gi, "&")
            .replace(/&lt;/gi, "<")
            .replace(/&gt;/gi, ">");
    let root = null;
    if (typeof DOMParser === "function") {
        root = new DOMParser().parseFromString(raw, "text/html").body;
    }
    if (!root) {
        return normalise(stripWithRegex());
    }
    const clone = root.cloneNode(true);
    clone.querySelectorAll("br").forEach((el) => el.replaceWith("\n"));
    ["p", "div", "tr", "li", "h1", "h2", "h3", "h4", "h5", "table", "section"].forEach((tag) => {
        clone.querySelectorAll(tag).forEach((el) => el.insertAdjacentText("afterend", "\n"));
    });
    return normalise(clone.textContent || "");
}

export async function textReportToPngBytes(text) {
    const FONT_PX = 13;
    const LINE_H = 17;
    const PAD = 28;
    const WIDTH = 794;
    const scratch = document.createElement("canvas").getContext("2d");
    scratch.font = FONT_PX + "px 'Courier New', monospace";
    const charW = scratch.measureText("M").width || 7.8;
    const maxChars = Math.max(20, Math.floor((WIDTH - PAD * 2) / charW));
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
    const height = Math.min(32000, PAD * 2 + lines.length * LINE_H);
    const canvas = document.createElement("canvas");
    canvas.width = WIDTH;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, WIDTH, height);
    ctx.fillStyle = "#000000";
    ctx.font = FONT_PX + "px 'Courier New', monospace";
    ctx.textBaseline = "top";
    lines.forEach((line, idx) => {
        const y = PAD + idx * LINE_H;
        if (y < height - LINE_H) ctx.fillText(line, PAD, y);
    });
    const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
    if (!blob) throw new Error("text render failed");
    return Array.from(new Uint8Array(await blob.arrayBuffer()));
}

export async function watchJobToCompletion(bridge, jobId, meta, env) {
    const pollJobId = (meta && meta.bridgeJobId) || jobId;
    let lastState = "queued";
    let lastPages = meta.pages || 1;
    let lastAttempts = 1;
    let consecutiveErrors = 0;
    let consecutiveMissing = 0;
    // One failure notification per (state, attempts): the watch
    // continues past retryable outcomes, so without this a job
    // failing on every retry would toast on every poll.
    let notifiedFailure = null;
    // Give up only after 5 minutes WITHOUT any state progress (a job
    // queued behind a busy desktop queue legitimately takes longer),
    // with a 30-minute absolute cap. The watcher is the only status
    // source for the server record: a fixed deadline silently left
    // long jobs non-terminal on the server forever.
    let lastProgressAt = Date.now();
    const absoluteDeadline = Date.now() + 30 * 60 * 1000;
    for (let i = 0; Date.now() < absoluteDeadline && Date.now() - lastProgressAt < 5 * 60 * 1000; i++) {
        await new Promise((resolve) =>
            setTimeout(resolve, i === 0 ? 50 : (i < 4 ? 150 : (i < 12 ? 400 : 1500))));
        try {
            let state = null;
            let errorCode = null;
            let pages = lastPages;
            let attempts = lastAttempts;
            if (typeof bridge.jobDetail === "function") {
                const detail = await bridge.jobDetail(pollJobId);
                if (detail) {
                    state = normalizeBridgeJobState(detail.state);
                    errorCode = detail.errorCode || detail.error_code || null;
                    pages = detail.pages || pages;
                    attempts = detail.attempts || attempts;
                }
            } else if (typeof bridge.printStatus === "function") {
                state = normalizeBridgeJobState(await bridge.printStatus(pollJobId));
            }
            consecutiveErrors = 0;
            if (!state) {
                if (++consecutiveMissing >= 15) {
                    break;
                }
                continue;
            }
            consecutiveMissing = 0;
            // Quiet retry check: if the desktop worker hit a transient
            // error (busy/offline) and auto-requeued or recovered within
            // one brief tick, resolve the recheck BEFORE posting `failed`
            // or flashing a sticky error toast so temporary retries stay
            // silent and the server log always receives the final state.
            if (state === "failed" && notifiedFailure !== state + ":" + attempts && typeof bridge.jobDetail === "function") {
                await new Promise((resolve) => setTimeout(resolve, 900));
                const recheck = await bridge.jobDetail(pollJobId).catch(() => null);
                const recheckState = recheck && normalizeBridgeJobState(recheck.state);
                if (recheckState) {
                    state = recheckState;
                    errorCode = recheck.errorCode || recheck.error_code || null;
                    pages = recheck.pages || pages;
                    attempts = recheck.attempts || attempts;
                }
            }
            if (state !== lastState || pages !== lastPages || attempts !== lastAttempts) {
                lastState = state;
                lastPages = pages;
                lastAttempts = attempts;
                lastProgressAt = Date.now();
                const statusPayload = {
                    idempotency_key: jobId,
                    state,
                    error_code: errorCode,
                    device: meta.printerName || meta.printerPattern,
                    report_name: meta.reportName,
                    source_type: meta.sourceType || "report",
                    pages,
                    byte_size: meta.byteSize,
                    attempts,
                };
                if (meta.sourceModel) statusPayload.source_model = meta.sourceModel;
                if (meta.sourceIds) statusPayload.source_ids = meta.sourceIds;
                if (meta.copies) statusPayload.copies = meta.copies;
                if (meta.paper) statusPayload.paper = meta.paper;
                if (typeof meta.duplex === "boolean") statusPayload.duplex = meta.duplex;
                if (typeof meta.color === "boolean") statusPayload.color = meta.color;
                if (meta.mode) statusPayload.mode = meta.mode;
                await rpc(
                    "/native_print/job_status",
                    statusPayload,
                    { silent: true },
                ).catch(() => {});
            }
            if (TERMINAL_STATES.has(state)) {
                const printerLabel = meta.printerName || meta.printerPattern;
                if (state === "done" || state === "canceled") {
                    if (state === "done") {
                        // "Done" means the desktop backend accepted and
                        // completed its submission stage; it is not proof
                        // paper physically emerged on every transport.
                        notifyUser(
                            env,
                            _t('Sent "%s" to %s', meta.reportName, printerLabel),
                            "success",
                        );
                    }
                    notifyJobUpdated();
                    return state;
                }
                const failureKey = state + ":" + attempts;
                if (notifiedFailure !== failureKey) {
                    notifiedFailure = failureKey;
                    notifyJobUpdated();
                    notifyUser(
                        env,
                        licenseErrorMessage(errorCode) || _t('Printing failed on "%s" (%s). Check the printer before reprinting.', printerLabel, safeErrorCode(errorCode, state)),
                        "danger",
                        undefined,
                        recoveryButtons(),
                    );
                }
                if (state === "dead_letter") {
                    return state;
                }
            }
        } catch (_err) {
            // One flaky poll must not orphan the job: the user would
            // get no outcome and the server log no terminal state.
            // Give up only after several failures in a row; the poll
            // ladder itself is unchanged.
            if (++consecutiveErrors >= 10) {
                break;
            }
        }
    }
    if (!TERMINAL_STATES.has(lastState)) {
        notifyUser(
            env,
            _t('Print job "%s" is still %s. Check Print Jobs for the final status.', meta.reportName, lastState),
            "warning",
            undefined,
            recoveryButtons(),
        );
    }
    return lastState;
}

async function handleCloseOnReportDownload(action, options, env) {
    const onClose = options && options.onClose;
    try {
        if (action.close_on_report_download) {
            if (env && env.services && env.services.action) {
                await env.services.action.doAction(
                    { type: "ir.actions.act_window_close" },
                    { onClose },
                );
                return;
            }
        }
        if (typeof onClose === "function") {
            onClose();
        }
    } catch (err) {
        // The print already succeeded at this point: a failing
        // wizard close (stale dialog, finished action service) must
        // not surface as a print error or an RPC traceback.
        console.warn("Itbaa: closing the report wizard failed after a successful print", err);
    }
}

/*
 * Odoo's PDF engine (wkhtmltopdf) is optional on the server. When it is
 * absent, /report/pdf/<name> answers with the HTML report — Odoo shows the
 * "Unable to find the PDF engine" warning and falls back to HTML. Sending
 * that HTML to the printer would print markup, so render the SAME report
 * through Odoo's text converter and print it natively instead. No server
 * PDF engine and no browser print dialog are ever involved.
 */
async function printReportTextWhenPdfEngineMissing(action, options, env, bridge, target, rule) {
    let text;
    try {
        text = await fetchReportBody(buildReportUrl(action, "text", env), (resp) => resp.text());
    } catch (_fetchError) {
        return undefined;
    }
    if (text === null) {
        return undefined;
    }
    // Odoo's text converter renders QWeb, so the body is normally HTML.
    const plain = reportHtmlToPlainText(text);
    if (!plain || /^\s*</.test(plain)) {
        return undefined;
    }
    text = plain;
    const textBytes = new TextEncoder().encode(text).length;
    const asEscpos = rule.mode === "escpos";
    const limit = asEscpos ? MAX_NATIVE_TEXT_BYTES : MAX_NATIVE_PAYLOAD_BYTES;
    if (!textBytes || textBytes > limit) {
        return undefined;
    }

    const key = buildReportIdempotencyKey("odoo-no-pdf", action);
    const source = reportSource(action);
    const meta = {
        reportName: action.report_name,
        printerPattern: rule.printer_pattern,
        printerName: target.printer.name,
        sourceType: "report",
        sourceModel: source.model,
        sourceIds: source.ids || undefined,
        copies: rule.copies || 1,
        paper: rule.paper || undefined,
        color: rule.color !== false,
        mode: asEscpos ? "escpos" : "gdi",
        pages: 1,
    };
    try {
        let enqueueRes;
        if (asEscpos) {
            enqueueRes = await bridge.printText({
                idempotencyKey: key,
                reportName: action.report_name,
                printer: target.printer.name,
                printerPattern: rule.printer_pattern,
                text,
                cut: true,
                copies: rule.copies || 1,
            });
        } else {
            const pngBytes = await textReportToPngBytes(text);
            enqueueRes = await bridge.printImage({
                idempotencyKey: key,
                reportName: action.report_name,
                printer: target.printer.name,
                printerPattern: rule.printer_pattern,
                mode: "gdi",
                sourceWidthMm: 210,
                copies: rule.copies || 1,
                bytes: pngBytes,
            });
        }
        const bridgeJobId = resolveBridgeJobId(enqueueRes, key);
        rpc("/native_print/job_status", Object.assign({
            idempotency_key: key,
            state: "queued",
            device: target.printer.name,
            report_name: action.report_name,
            byte_size: textBytes,
        }, source.model ? { source_model: source.model } : {}, source.ids ? { source_ids: source.ids } : {}), { silent: true }).catch(() => {});
        watchJobToCompletion(
            bridge,
            key,
            Object.assign({}, meta, { byteSize: textBytes, bridgeJobId }),
            env,
        ).catch(() => {});
    } catch (error) {
        const licenseCode = extractLicenseErrorCode(error);
        rpc("/native_print/job_status", Object.assign({
            idempotency_key: key,
            state: "failed",
            error_code: licenseCode || "BRIDGE_ERROR",
            device: target.printer.name,
            report_name: action.report_name,
            byte_size: textBytes,
        }, source.model ? { source_model: source.model } : {}, source.ids ? { source_ids: source.ids } : {}), { silent: true })
            .catch(() => {})
            .finally(() => notifyJobUpdated());
        console.warn("Itbaa: text fallback for a report without a PDF engine failed.", error);
        if (licenseCode) {
            notifyUser(env, licenseErrorMessage(licenseCode), "danger", undefined, recoveryButtons());
        }
        return undefined;
    }
    notifyUser(
        env,
        _t("The Odoo server has no PDF engine. The report was printed from its text version instead."),
        "warning",
        undefined,
        recoveryButtons(),
    );
    await handleCloseOnReportDownload(action, options, env);
    return true;
}

async function handleReportSubmission(action, options, env, bridge) {
    // A reprint launched from the job log carries a prefill: the
    // original printer and the original print settings. The printer
    // is preferred when it is connected to this PC; the settings
    // override today's defaults for this print only.
    const prefill = (action.context && action.context.native_print_prefill) || null;
    const targetPromise = resolveBridgeTarget(bridge, {
        preferPrinterName: (prefill && prefill.printer) || undefined,
    }).catch(() => null);

    const applyPrefillAndFinishTarget = (target) => {
        if (!target) {
            // Notify inside the desktop app so falling back to download is explicit.
            notifyUser(
                env,
                _t("No printer is available on this device right now. The file was downloaded instead."),
                "warning",
                undefined,
                recoveryButtons(),
            );
            return null;
        }
        const rule = target.rule;
        if (prefill && prefill.settings) {
            const st = prefill.settings;
            if (Number.isInteger(st.copies) && st.copies > 0) rule.copies = st.copies;
            if (typeof st.paper === "string" && st.paper) rule.paper = st.paper;
            if (typeof st.duplex === "boolean") rule.duplex = st.duplex;
            if (typeof st.color === "boolean") rule.color = st.color;
        }
        return { target, rule };
    };

    if (action.report_type === "qweb-html") {
        // Odoo already converted the action to HTML because the server has
        // no PDF engine: print it natively from its text rendering.
        const resolved = applyPrefillAndFinishTarget(await targetPromise);
        if (!resolved) return undefined;
        return await printReportTextWhenPdfEngineMissing(action, options, env, bridge, resolved.target, resolved.rule);
    }

    if (action.report_type === "qweb-text") {
        const textUrl = buildReportUrl(action, "text", env);
        const textPromise = fetchReportBody(textUrl, (resp) => resp.text()).catch(() => null);
        const [rawTarget, text] = await Promise.all([targetPromise, textPromise]);
        const resolved = applyPrefillAndFinishTarget(rawTarget);
        if (!resolved) return undefined;
        const { target, rule } = resolved;
        if (text === null) {
            // Let Odoo's default flow handle (and report) it.
            return undefined;
        }
        // Byte size means UTF-8 bytes (what actually crosses the
        // bridge), not UTF-16 code units: text.length undercounts
        // Arabic/CJK content for the job log and the size guard.
        const textBytes = new TextEncoder().encode(text).length;
        // Thermal and label targets accept raw text streams (ESC/POS or
        // ZPL/TSPL/EPL/CPCL); office/virtual targets receive a GDI PNG.
        const textAsRaw = ["escpos", "zpl", "tspl", "epl", "cpcl"].includes(rule.mode);
        const maxTextBytes = textAsRaw ? MAX_NATIVE_TEXT_BYTES : MAX_NATIVE_PAYLOAD_BYTES;
        if ((textAsRaw && textBytes === 0) || textBytes > maxTextBytes) {
            // Empty or oversized for the desktop queue: keep Odoo's standard
            // download rather than a doomed enqueue.
            return undefined;
        }
        const textKey = buildReportIdempotencyKey("odoo-txt", action);
        const textSource = reportSource(action);
        const textReportData = reportDataOf(action);
        try {
            let textEnqueueRes;
            if (textAsRaw) {
                textEnqueueRes = await bridge.printText({
                    idempotencyKey: textKey,
                    reportName: action.report_name,
                    printer: target.printer.name,
                    printerPattern: rule.printer_pattern,
                    mode: rule.mode,
                    text,
                    cut: rule.mode === "escpos",
                    copies: rule.copies || 1,
                });
            } else {
                const pngBytes = await textReportToPngBytes(text);
                textEnqueueRes = await bridge.printImage({
                    idempotencyKey: textKey,
                    reportName: action.report_name,
                    printer: target.printer.name,
                    printerPattern: rule.printer_pattern,
                    mode: "gdi",
                    sourceWidthMm: 210,
                    copies: rule.copies || 1,
                    bytes: pngBytes,
                });
            }
            const textBridgeJobId = resolveBridgeJobId(textEnqueueRes, textKey);
            await Promise.race([
                rpc("/native_print/job_status", {
                    idempotency_key: textKey,
                    state: "queued",
                    device: target.printer.name,
                    report_name: action.report_name,
                    source_type: "report",
                    source_model: textSource.model,
                    source_ids: textSource.ids || undefined,
                    ...(textReportData ? { report_data: textReportData } : {}),
                    copies: rule.copies || 1,
                    paper: rule.paper || undefined,
                    color: rule.color !== false,
                    mode: textAsRaw ? rule.mode : "gdi",
                    pages: 1,
                    byte_size: textBytes,
                }, { silent: true }).catch(() => {}),
                new Promise((r) => setTimeout(r, 400)),
            ]);
            watchJobToCompletion(
                bridge,
                textKey,
                {
                    bridgeJobId: textBridgeJobId,
                    reportName: action.report_name,
                    printerPattern: rule.printer_pattern,
                    printerName: target.printer.name,
                    sourceType: "report",
                    sourceModel: textSource.model,
                    sourceIds: textSource.ids || undefined,
                    copies: rule.copies || 1,
                    paper: rule.paper || undefined,
                    color: rule.color !== false,
                    mode: textAsRaw ? rule.mode : "gdi",
                    byteSize: textBytes,
                    pages: 1,
                },
                env,
            ).catch(() => {});
        } catch (error) {
            const licenseCode = extractLicenseErrorCode(error);
            // Logging the failure is best-effort: if this rpc threw,
            // the exception would escape the fallback path, the user
            // would get an RPC error instead of the file, and the
            // download below would never happen.
            rpc("/native_print/job_status", {
                idempotency_key: textKey,
                state: "failed",
                error_code: licenseCode || "BRIDGE_ERROR",
                device: target.printer.name,
                report_name: action.report_name,
                source_type: "report",
                source_model: textSource.model,
                source_ids: textSource.ids || undefined,
                ...(textReportData ? { report_data: textReportData } : {}),
                copies: rule.copies || 1,
                paper: rule.paper || undefined,
                color: rule.color !== false,
                mode: textAsRaw ? rule.mode : "gdi",
                pages: 1,
                byte_size: textBytes,
            }, { silent: true })
                .catch(() => {})
                .finally(() => notifyJobUpdated());
            console.warn("Itbaa: text print bridge call failed.", error);
            notifyUser(
                env,
                licenseErrorMessage(licenseCode) || _t('Itbaa could not print on "%s". The file was downloaded instead.', target.printer.name),
                licenseCode ? "danger" : "warning",
                undefined,
                recoveryButtons(),
            );
            return undefined; // Fall back to the standard download.
        }
        await handleCloseOnReportDownload(action, options, env);
        return true;
    }

    const url = buildReportUrl(action, "pdf", env);
    const pdfPromise = fetchReportBody(url, (resp) => resp.arrayBuffer())
        .then((buf) => ({ ok: true, buf }))
        .catch(() => ({ ok: false, buf: null }));
    const [rawTarget, fetchOutcome] = await Promise.all([targetPromise, pdfPromise]);
    const resolved = applyPrefillAndFinishTarget(rawTarget);
    if (!resolved) {
        return undefined;
    }
    const { target, rule } = resolved;
    if (!fetchOutcome.ok || fetchOutcome.buf === null) {
        // Let Odoo's default flow handle (and report) it.
        return undefined;
    }
    const rawBytes = new Uint8Array(fetchOutcome.buf);
    if (rawBytes.length > MAX_NATIVE_PAYLOAD_BYTES) {
        // Above desktop queue limit: fall back to download.
        return undefined;
    }
    // A 200 response is not necessarily a PDF: an expired session
    // answers with the login page's HTML. Never queue HTML as a
    // document; fall back to the standard download instead.
    const hasPdfMagic = (() => {
        const limit = Math.min(rawBytes.length - 3, 16);
        for (let i = 0; i < limit; i++) {
            if (rawBytes[i] === 0x25 && rawBytes[i + 1] === 0x50 && rawBytes[i + 2] === 0x44 && rawBytes[i + 3] === 0x46) {
                return true;
            }
        }
        return false;
    })();
    if (!hasPdfMagic) {
        // Odoo returned HTML because its PDF engine is missing. Print the
        // same report natively from the text converter instead of letting
        // Odoo open the HTML fallback in the browser.
        return await printReportTextWhenPdfEngineMissing(action, options, env, bridge, target, rule);
    }
    // Pass Uint8Array directly so BRIDGE_JS encodes it as base64 rather than
    // expanding every byte into a huge JSON integer array.
    const bytes = rawBytes;
    const idempotencyKey = buildReportIdempotencyKey("odoo", action);
    const pdfSource = reportSource(action);
    const pdfReportData = reportDataOf(action);
    try {
        const pdfEnqueueRes = await bridge.printPdf({
            idempotencyKey,
            reportName: action.report_name,
            printer: target.printer.name,
            printerPattern: rule.printer_pattern,
            mode: rule.mode,
            paper: rule.paper || null,
            copies: rule.copies,
            // Tri-state: null leaves the driver default in
            // charge; only an explicit choice (reprint prefill)
            // or the app's saved per-printer default may set it.
            duplex: typeof rule.duplex === "boolean" ? rule.duplex : null,
            color: rule.color !== false,
            tray: rule.tray || null,
            scalePolicy: rule.scale_policy,
            mediaRollWidthMm:
                (rule.media && rule.media.roll_width_mm) || undefined,
            bytes,
        });
        const pdfBridgeJobId = resolveBridgeJobId(pdfEnqueueRes, idempotencyKey);
        await Promise.race([
            rpc("/native_print/job_status", {
                idempotency_key: idempotencyKey,
                state: "queued",
                device: target.printer.name,
                report_name: action.report_name,
                source_type: "report",
                source_model: pdfSource.model,
                source_ids: pdfSource.ids || undefined,
                ...(pdfReportData ? { report_data: pdfReportData } : {}),
                copies: rule.copies || 1,
                paper: rule.paper || undefined,
                duplex: typeof rule.duplex === "boolean" ? rule.duplex : null,
                color: rule.color !== false,
                mode: rule.mode || undefined,
                byte_size: bytes.length,
            }, { silent: true }).catch(() => {}),
            new Promise((r) => setTimeout(r, 400)),
        ]);
        watchJobToCompletion(
            bridge,
            idempotencyKey,
            {
                bridgeJobId: pdfBridgeJobId,
                reportName: action.report_name,
                printerPattern: rule.printer_pattern,
                printerName: target.printer.name,
                sourceType: "report",
                sourceModel: pdfSource.model,
                sourceIds: pdfSource.ids || undefined,
                copies: rule.copies || 1,
                paper: rule.paper || undefined,
                duplex: typeof rule.duplex === "boolean" ? rule.duplex : null,
                color: rule.color !== false,
                mode: rule.mode || undefined,
                byteSize: bytes.length,
                pages: 1,
            },
            env,
        ).catch(() => {});
    } catch (error) {
        const licenseCode = extractLicenseErrorCode(error);
        // Best-effort like the text path: never let failure logging
        // break the fallback download (see the qweb-text branch).
        rpc("/native_print/job_status", {
            idempotency_key: idempotencyKey,
            state: "failed",
            error_code: licenseCode || "BRIDGE_ERROR",
            device: target.printer.name,
            report_name: action.report_name,
            source_type: "report",
            source_model: pdfSource.model,
            source_ids: pdfSource.ids || undefined,
            ...(pdfReportData ? { report_data: pdfReportData } : {}),
            copies: rule.copies || 1,
            paper: rule.paper || undefined,
            duplex: typeof rule.duplex === "boolean" ? rule.duplex : null,
            color: rule.color !== false,
            mode: rule.mode || undefined,
            byte_size: bytes.length,
        }, { silent: true })
            .catch(() => {})
            .finally(() => notifyJobUpdated());
        console.warn("Itbaa: PDF print bridge call failed.", error);
        notifyUser(
            env,
            licenseErrorMessage(licenseCode) || _t('Itbaa could not print on "%s". The PDF was downloaded instead.', target.printer.name),
            licenseCode ? "danger" : "warning",
            undefined,
            recoveryButtons(),
        );
        return undefined; // Fall back to the standard download.
    }
    await handleCloseOnReportDownload(action, options, env);
    return true; // Skip Odoo's default download.
}

registry.category("ir.actions.report handlers").add(
    "native_print",
    async function (action, options, env) {
        const bridge = nativePrintBridge();
        if (!bridge) {
            return undefined; // Normal browser: unchanged behaviour.
        }
        const reportType = String((action && action.report_type) || "");
        // Odoo can hand us an engine-specific qweb-pdf-* type, or an
        // already-converted qweb-html action when no PDF engine exists.
        // All of them must stay inside the native print path.
        const supported =
            reportType === "qweb-text" ||
            reportType === "qweb-html" ||
            reportType.startsWith("qweb-pdf");
        if (!action || !supported) {
            return undefined; // Other report types keep the default flow.
        }
        const dataFingerprint = (() => {
            try {
                const raw = JSON.stringify(action.data || "") +
                    "|" + JSON.stringify(action.ids || action.res_id || "");
                let h = 0;
                for (let i = 0; i < raw.length; i++) {
                    h = (h * 31 + raw.charCodeAt(i)) | 0;
                }
                return String(h);
            } catch (_e) {
                return "";
            }
        })();
        const guardKey =
            `${action.report_name || action.report_url || ""}|` +
            `${action.report_type || ""}|` +
            JSON.stringify(
                (action.context &&
                    (action.context.active_ids ||
                        action.context.active_id)) ||
                    "",
            ) +
            `|${dataFingerprint}`;
        const now = Date.now();
        const existing = inFlightReports.get(guardKey);
        if (existing && now - existing.startedAt < IN_FLIGHT_STALE_MS) {
            // The same report for the same records is already
            // being submitted (double-click, key repeat): the
            // duplicate must not print twice.
            return true;
        }
        const token = { startedAt: now };
        inFlightReports.set(guardKey, token);
        if (inFlightReports.size > 20) {
            for (const [k, entry] of inFlightReports) {
                if (now - entry.startedAt >= IN_FLIGHT_STALE_MS) {
                    inFlightReports.delete(k);
                }
            }
        }
        try {
            return await handleReportSubmission(action, options, env, bridge);
        } finally {
            if (inFlightReports.get(guardKey) === token) {
                inFlightReports.delete(guardKey);
            }
        }
    },
);

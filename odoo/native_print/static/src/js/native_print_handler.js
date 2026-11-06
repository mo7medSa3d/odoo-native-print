/** @odoo-module **/
import { registry } from "@web/core/registry";
import { rpc } from "@web/core/network/rpc";
import { _t } from "@web/core/l10n/translation";
import { notifyUser, resolveBridgeTarget } from "./native_print_client";

/*
 * Native Print report handler (API contract v1, docs/api-contract.md).
 *
 * - Normal browser: no bridge exists -> return undefined (falsy) and Odoo
 *   behaves exactly as usual. This is the most important line in the file.
 * - Inside the Native Print app the shell injects window.__NATIVE_PRINT__
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

const TERMINAL_STATES = new Set(["done", "failed", "canceled", "dead_letter"]);
// Only these end the watch. `failed` and `dead_letter` are
// retryable outcomes (the desktop worker re-queues a failed job
// and the operator may retry a dead-letter job): the watcher
// keeps polling after them so the retry's outcome, not the
// first failure, is what the user and the server log end with.

// The desktop queue's hard payload ceiling (print-core MAX_JOB_BYTES,
// 50 MB). Payloads above it can never be accepted, so the handler
// checks before enqueueing and keeps Odoo's standard download.
const MAX_NATIVE_PAYLOAD_BYTES = 50 * 1024 * 1024;

// Report submissions currently between click and desktop enqueue,
// keyed by report + target records. A second invocation for the same
// key is a duplicate of the SAME user intent, not a new print.
const inFlightReports = new Set();

export function nativePrintBridge() {
    return window.__NATIVE_PRINT__ || null;
}

export function buildReportUrl(action, converter = "pdf", env = null) {
    let url = `/report/${converter}/${action.report_name}`;
    const actionContext = action.context || {};
    const userContext = (env && env.services && env.services.user && env.services.user.context) || {};
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
        const ids =
            actionContext.active_ids ||
            (actionContext.active_id ? [actionContext.active_id] : null) ||
            action.ids ||
            (action.res_id ? [action.res_id] : []);
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

// The source reference (model + record ids) a report is printed
// for, posted with the job so the job log can offer a real Reprint.
export function reportSource(action) {
    const ctx = (action && action.context) || {};
    const ids =
        ctx.active_ids ||
        (ctx.active_id ? [ctx.active_id] : null) ||
        action.ids ||
        (action.res_id ? [action.res_id] : []);
    const list = Array.isArray(ids) ? ids : [ids];
    return {
        model: ctx.active_model || action.model || null,
        ids: list.filter((x) => Number.isInteger(x) || /^\d+$/.test(String(x))).join(","),
    };
}

/*
 * Render a plain-text report as a PNG document (A4 width at
 * 96 dpi, monospace, hard-wrapped) so it can print through GDI
 * on office, label and virtual printers, which cannot interpret
 * ESC/POS (2026-11-07). The desktop canvas path paginates a tall
 * image across sheets, so one long PNG is the whole document.
 * Throws when the canvas is unavailable; the caller falls back
 * to Odoo's standard download.
 */
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
    let lastState = "queued";
    let lastPages = meta.pages || 1;
    let lastAttempts = 1;
    let consecutiveErrors = 0;
    // One failure notification per (state, attempts): the watch
    // continues past retryable outcomes, so without this a job
    // failing on every retry would toast on every poll.
    let notifiedFailure = null;
    // Give up only after 5 minutes WITHOUT any state progress (a job
    // queued behind a busy desktop queue legitimately takes longer),
    // with a 30-minute absolute cap. The watcher is the only status
    // source for the server record: a fixed deadline silently left
    // long jobs non-terminal on the server forever. (Audit F7.)
    let lastProgressAt = Date.now();
    const absoluteDeadline = Date.now() + 30 * 60 * 1000;
    for (let i = 0; Date.now() < absoluteDeadline && Date.now() - lastProgressAt < 5 * 60 * 1000; i++) {
        await new Promise((resolve) =>
            setTimeout(resolve, i < 10 ? 600 : 2000));
        try {
            let state = null;
            let errorCode = null;
            let pages = lastPages;
            let attempts = lastAttempts;
            if (typeof bridge.jobDetail === "function") {
                const detail = await bridge.jobDetail(jobId);
                if (detail) {
                    state = detail.state;
                    errorCode = detail.errorCode || null;
                    pages = detail.pages || pages;
                    attempts = detail.attempts || attempts;
                }
            } else if (typeof bridge.printStatus === "function") {
                state = await bridge.printStatus(jobId);
            }
            consecutiveErrors = 0;
            if (!state) {
                continue;
            }
            if (state !== lastState || pages !== lastPages || attempts !== lastAttempts) {
                lastState = state;
                lastPages = pages;
                lastAttempts = attempts;
                lastProgressAt = Date.now();
                await rpc(
                    "/native_print/job_status",
                    {
                        idempotency_key: jobId,
                        state,
                        error_code: errorCode,
                        device: meta.printerName || meta.printerPattern,
                        report_name: meta.reportName,
                        pages,
                        byte_size: meta.byteSize,
                        attempts,
                    },
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
                    return state;
                }
                // Retryable outcome (failed / dead_letter): notify
                // once per attempt, then KEEP WATCHING. The worker
                // may re-queue the job; stopping here would leave
                // the retry's outcome unposted and the server log
                // stuck on the first failure. The existing
                // 5-minute-no-progress / 30-minute ladder bounds
                // the extended watch.
                const failureKey = state + ":" + attempts;
                if (notifiedFailure !== failureKey) {
                    notifiedFailure = failureKey;
                    notifyUser(
                        env,
                        _t('Printing failed on "%s" (%s). Check the printer before reprinting.', printerLabel, errorCode || state),
                        "danger",
                    );
                }
            }
        } catch (_err) {
            // One flaky poll must not orphan the job: the user would
            // get no outcome and the server log no terminal state.
            // Give up only after several failures in a row; the poll
            // ladder itself is unchanged.
            if (++consecutiveErrors >= 3) {
                break;
            }
        }
    }
    if (!TERMINAL_STATES.has(lastState)) {
        notifyUser(
            env,
            _t('Print job "%s" is still %s; its final outcome is unconfirmed. Check Print Jobs later.', meta.reportName, lastState),
            "warning",
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
        // not surface as a print error or an RPC traceback (audit
        // L6). Log it and let the handler return success.
        console.warn("Native Print: closing the report wizard failed after a successful print", err);
    }
}

async function handleReportSubmission(action, options, env, bridge) {
    // A reprint launched from the job log carries a prefill: the
    // original printer and the original print settings. The printer
    // is preferred when it is connected to this PC; the settings
    // override today's defaults for this print only.
    const prefill = (action.context && action.context.native_print_prefill) || null;
    const target = await resolveBridgeTarget(bridge, {
        preferPrinterName: (prefill && prefill.printer) || undefined,
    }).catch(() => null);
    if (!target) {
        // Inside the app, falling back to a downloaded file with
        // no word reads as "printing did nothing": say so first
        // (2026-11-05). In a normal browser there is no bridge and
        // this handler never runs, so the download stays silent
        // there as it should.
        notifyUser(
            env,
            _t("No printer is available on this device right now. The file was downloaded instead."),
            "warning",
        );
        return undefined; // No printer on this PC right now.
    }
    const rule = target.rule;
    if (prefill && prefill.settings) {
        const st = prefill.settings;
        if (Number.isInteger(st.copies) && st.copies > 0) rule.copies = st.copies;
        if (typeof st.paper === "string" && st.paper) rule.paper = st.paper;
        if (typeof st.duplex === "boolean") rule.duplex = st.duplex;
        if (typeof st.color === "boolean") rule.color = st.color;
    }

    if (action.report_type === "qweb-text") {
        const textUrl = buildReportUrl(action, "text", env);
        let text;
        try {
            const resp = await fetch(textUrl, { credentials: "same-origin" });
            if (!resp.ok) {
                // Let Odoo's default flow handle (and report) it.
                return undefined;
            }
            text = await resp.text();
        } catch (_fetchError) {
            // Network failure / server restart mid-click: the
            // fallback contract is "return undefined and Odoo
            // downloads as usual" — a rejected fetch must not escape
            // the handler as an exception. (Audit F1.)
            return undefined;
        }
        // Byte size means UTF-8 bytes (what actually crosses the
        // bridge), not UTF-16 code units: text.length undercounts
        // Arabic/CJK content for the job log and the size guard.
        const textBytes = new TextEncoder().encode(text).length;
        if (textBytes > MAX_NATIVE_PAYLOAD_BYTES) {
            // Oversized for the desktop queue: keep Odoo's standard
            // download rather than a doomed enqueue (audit L5).
            return undefined;
        }
        const textKey =
            `odoo-txt-${action.id || action.report_name}-${Date.now()}-` +
            Math.random().toString(36).slice(2, 10);
        const textSource = reportSource(action);
        // ESC/POS is a thermal printer's language (external
        // review, 2026-11-07): only a thermal target may receive
        // the text as ESC/POS. An office, label or virtual target
        // gets the same text rendered as a document image through
        // GDI — readable output on its own paper — never raw
        // bytes it cannot interpret. (The desktop worker refuses
        // raw payloads to office/virtual targets as a second line
        // of defence.)
        const textAsEscpos = rule.mode === "escpos";
        await rpc("/native_print/job_status", {
            idempotency_key: textKey,
            state: "queued",
            device: target.printer.name,
            report_name: action.report_name,
            source_type: "report",
            source_model: textSource.model,
            source_ids: textSource.ids || undefined,
            copies: rule.copies || 1,
            paper: rule.paper || undefined,
            duplex: !!rule.duplex,
            color: rule.color !== false,
            mode: textAsEscpos ? "escpos" : "gdi",
            pages: 1,
            byte_size: textBytes,
        }).catch(() => {});
        try {
            if (textAsEscpos) {
                await bridge.printText({
                    idempotencyKey: textKey,
                    reportName: action.report_name,
                    printerPattern: rule.printer_pattern,
                    text,
                    cut: true,
                    copies: rule.copies || 1,
                });
            } else {
                const pngBytes = await textReportToPngBytes(text);
                await bridge.printImage({
                    idempotencyKey: textKey,
                    reportName: action.report_name,
                    printerPattern: rule.printer_pattern,
                    mode: "gdi",
                    sourceWidthMm: 210,
                    copies: rule.copies || 1,
                    bytes: pngBytes,
                });
            }
            watchJobToCompletion(
                bridge,
                textKey,
                {
                    reportName: action.report_name,
                    printerPattern: rule.printer_pattern,
                    printerName: target.printer.name,
                    byteSize: textBytes,
                    pages: 1,
                },
                env,
            ).catch(() => {});
        } catch (error) {
            // Logging the failure is best-effort: if this rpc threw,
            // the exception would escape the fallback path, the user
            // would get an RPC error instead of the file, and the
            // download below would never happen.
            await rpc("/native_print/job_status", {
                idempotency_key: textKey,
                state: "failed",
                error_code: "BRIDGE_ERROR",
                device: target.printer.name,
            }, { silent: true }).catch(() => {});
            notifyUser(
                env,
                _t('Native Print could not print on "%s": %s. The file was downloaded instead.', target.printer.name, error.message || error),
                "warning",
            );
            return undefined; // Fall back to the standard download.
        }
        await handleCloseOnReportDownload(action, options, env);
        return true;
    }

    const url = buildReportUrl(action, "pdf", env);
    let bytes;
    try {
        const response = await fetch(url, { credentials: "same-origin" });
        if (!response.ok) {
            // Let Odoo's default flow handle (and report) it.
            return undefined;
        }
        bytes = Array.from(new Uint8Array(await response.arrayBuffer()));
    } catch (_fetchError) {
        // Same fallback contract as the text branch. (Audit F1.)
        return undefined;
    }
    // A 200 response is not necessarily a PDF: an expired session
    // answers with the login page's HTML. Never queue HTML as a
    // document; fall back to the standard download instead.
    if (!(bytes[0] === 0x25 && bytes[1] === 0x50 && bytes[2] === 0x44 && bytes[3] === 0x46)) {
        return undefined;
    }
    if (bytes.length > MAX_NATIVE_PAYLOAD_BYTES) {
        // Bigger than the desktop queue accepts: downloading keeps
        // the document usable instead of failing at the bridge
        // (audit L5).
        return undefined;
    }
    const idempotencyKey =
        `odoo-${action.id || action.report_name}-${Date.now()}-` +
        Math.random().toString(36).slice(2, 10);
    const pdfSource = reportSource(action);
    await rpc("/native_print/job_status", {
        idempotency_key: idempotencyKey,
        state: "queued",
        device: target.printer.name,
        report_name: action.report_name,
        source_type: "report",
        source_model: pdfSource.model,
        source_ids: pdfSource.ids || undefined,
        copies: rule.copies || 1,
        paper: rule.paper || undefined,
        duplex: !!rule.duplex,
        color: rule.color !== false,
        mode: rule.mode || undefined,
        byte_size: bytes.length,
    }).catch(() => {});
    try {
        await bridge.printPdf({
            idempotencyKey,
            reportName: action.report_name,
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
            url,
            bytes,
        });
        watchJobToCompletion(
            bridge,
            idempotencyKey,
            {
                reportName: action.report_name,
                printerPattern: rule.printer_pattern,
                printerName: target.printer.name,
                byteSize: bytes.length,
                pages: 1,
            },
            env,
        ).catch(() => {});
    } catch (error) {
        // Best-effort like the text path: never let failure logging
        // break the fallback download (see the qweb-text branch).
        await rpc("/native_print/job_status", {
            idempotency_key: idempotencyKey,
            state: "failed",
            error_code: "BRIDGE_ERROR",
            device: target.printer.name,
        }, { silent: true }).catch(() => {});
        notifyUser(
            env,
            _t('Native Print could not print on "%s": %s. The PDF was downloaded instead.', target.printer.name, error.message || error),
            "warning",
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
        if (!["qweb-pdf", "qweb-text"].includes(action.report_type)) {
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
            `${action.report_name}|` +
            JSON.stringify(
                (action.context &&
                    (action.context.active_ids ||
                        action.context.active_id)) ||
                    "",
            ) +
            `|${dataFingerprint}`;
        if (inFlightReports.has(guardKey)) {
            // The same report for the same records is already
            // being submitted (double-click, key repeat): the
            // duplicate must not print twice.
            return true;
        }
        inFlightReports.add(guardKey);
        try {
            return await handleReportSubmission(action, options, env, bridge);
        } finally {
            inFlightReports.delete(guardKey);
        }
    },
);

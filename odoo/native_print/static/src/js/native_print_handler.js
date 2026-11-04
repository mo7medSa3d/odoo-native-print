/** @odoo-module **/
import { registry } from "@web/core/registry";
import { rpc } from "@web/core/network/rpc";
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

export async function watchJobToCompletion(bridge, jobId, meta, env) {
    let lastState = "queued";
    let lastPages = meta.pages || 1;
    let lastAttempts = 1;
    let consecutiveErrors = 0;
    const deadline = Date.now() + 5 * 60 * 1000;
    for (let i = 0; Date.now() < deadline; i++) {
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
                await rpc(
                    "/native_print/job_status",
                    {
                        idempotency_key: jobId,
                        state,
                        error_code: errorCode,
                        device: meta.printerPattern,
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
                if (state === "done") {
                    notifyUser(
                        env,
                        `Printed "${meta.reportName}" on ${printerLabel}`,
                        "success",
                    );
                } else if (state === "failed" || state === "dead_letter") {
                    notifyUser(
                        env,
                        `Printing failed on "${printerLabel}" (${errorCode || state}). Nothing was printed.`,
                        "danger",
                    );
                }
                return state;
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
    return lastState;
}

async function handleCloseOnReportDownload(action, options, env) {
    const onClose = options && options.onClose;
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
        const resp = await fetch(textUrl, { credentials: "same-origin" });
        if (!resp.ok) {
            // Let Odoo's default flow handle (and report) it.
            return undefined;
        }
        const text = await resp.text();
        const textKey =
            `odoo-txt-${action.id || action.report_name}-${Date.now()}-` +
            Math.random().toString(36).slice(2, 10);
        const textSource = reportSource(action);
        await rpc("/native_print/job_status", {
            idempotency_key: textKey,
            state: "queued",
            device: rule.printer_pattern,
            report_name: action.report_name,
            source_type: "report",
            source_model: textSource.model,
            source_ids: textSource.ids || undefined,
            copies: rule.copies || 1,
            paper: rule.paper || undefined,
            duplex: !!rule.duplex,
            color: rule.color !== false,
            mode: rule.mode || undefined,
            pages: 1,
            byte_size: text.length,
        }).catch(() => {});
        try {
            await bridge.printText({
                idempotencyKey: textKey,
                reportName: action.report_name,
                printerPattern: rule.printer_pattern,
                text,
                cut: true,
                copies: rule.copies || 1,
            });
            watchJobToCompletion(
                bridge,
                textKey,
                {
                    reportName: action.report_name,
                    printerPattern: rule.printer_pattern,
                    printerName: target.printer.name,
                    byteSize: text.length,
                    pages: 1,
                },
                env,
            );
        } catch (error) {
            // Logging the failure is best-effort: if this rpc threw,
            // the exception would escape the fallback path, the user
            // would get an RPC error instead of the file, and the
            // download below would never happen.
            await rpc("/native_print/job_status", {
                idempotency_key: textKey,
                state: "failed",
                error_code: "BRIDGE_ERROR",
                device: rule.printer_pattern,
            }, { silent: true }).catch(() => {});
            notifyUser(
                env,
                `Native Print could not print on "${target.printer.name}": ${error.message || error}. The file was downloaded instead.`,
                "warning",
            );
            return undefined; // Fall back to the standard download.
        }
        await handleCloseOnReportDownload(action, options, env);
        return true;
    }

    const url = buildReportUrl(action, "pdf", env);
    const response = await fetch(url, { credentials: "same-origin" });
    if (!response.ok) {
        // Let Odoo's default flow handle (and report) it.
        return undefined;
    }
    const bytes = Array.from(new Uint8Array(await response.arrayBuffer()));
    const idempotencyKey =
        `odoo-${action.id || action.report_name}-${Date.now()}-` +
        Math.random().toString(36).slice(2, 10);
    const pdfSource = reportSource(action);
    await rpc("/native_print/job_status", {
        idempotency_key: idempotencyKey,
        state: "queued",
        device: rule.printer_pattern,
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
            duplex: !!rule.duplex,
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
        );
    } catch (error) {
        // Best-effort like the text path: never let failure logging
        // break the fallback download (see the qweb-text branch).
        await rpc("/native_print/job_status", {
            idempotency_key: idempotencyKey,
            state: "failed",
            error_code: "BRIDGE_ERROR",
            device: rule.printer_pattern,
        }, { silent: true }).catch(() => {});
        notifyUser(
            env,
            `Native Print could not print on "${target.printer.name}": ${error.message || error}. The PDF was downloaded instead.`,
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
        const guardKey =
            `${action.report_name}|` +
            JSON.stringify(
                (action.context &&
                    (action.context.active_ids ||
                        action.context.active_id)) ||
                    "",
            );
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

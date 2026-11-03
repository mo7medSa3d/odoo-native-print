/** @odoo-module **/
import { registry } from "@web/core/registry";
import { rpc } from "@web/core/network/rpc";

/*
 * Native Print report handler (API contract v1, docs/api-contract.md).
 *
 * - Normal browser: no bridge exists -> return undefined (falsy) and Odoo
 *   behaves exactly as usual. This is the most important line in the file.
 * - Inside the Native Print app the shell injects window.__NATIVE_PRINT__
 *   on the configured origin only. We then ask the SERVER which rule and
 *   profile apply (policy is never invented client-side), fetch the PDF
 *   with the user's own session, hand the raw bytes to the bridge, watch
 *   the local worker status until terminal, report the job state back,
 *   and return true so Odoo skips its download.
 * - If the server says there is no rule for this report, we also return
 *   undefined: native printing is opt-in per report, by configuration.
 */

const TERMINAL_STATES = new Set(["done", "failed", "canceled", "dead_letter"]);

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

function notifyUser(env, message, type = "info", title = "Native Print") {
    try {
        if (env && env.services && env.services.notification) {
            env.services.notification.add(message, { title, type });
        }
    } catch (_err) {
        // Notification is informational; never block printing.
    }
}

export async function watchJobToCompletion(bridge, jobId, meta, env) {
    let lastState = "queued";
    let lastPages = meta.pages || 1;
    let lastAttempts = 1;
    for (let i = 0; i < 40; i++) {
        await new Promise((resolve) => setTimeout(resolve, 600));
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
                        profile_id: meta.profileId,
                        pages,
                        byte_size: meta.byteSize,
                        attempts,
                    },
                    { silent: true },
                ).catch(() => {});
            }
            if (TERMINAL_STATES.has(state)) {
                if (state === "done") {
                    notifyUser(
                        env,
                        `Printed "${meta.reportName}" on ${meta.printerPattern}`,
                        "success",
                    );
                } else if (state === "failed" || state === "dead_letter") {
                    notifyUser(
                        env,
                        `Print failed on ${meta.printerPattern} (${errorCode || state})`,
                        "danger",
                    );
                }
                return state;
            }
        } catch (_err) {
            break;
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
        const resolved = await rpc("/native_print/rules", {
            report_name: action.report_name,
        });
        if (!resolved || !resolved.rule) {
            return undefined; // No native rule configured for this report.
        }
        const rule = resolved.rule;

        if (action.report_type === "qweb-text") {
            const textUrl = buildReportUrl(action, "text", env);
            const resp = await fetch(textUrl, { credentials: "same-origin" });
            if (!resp.ok) {
                throw new Error(`Native Print: text report fetch failed (${resp.status})`);
            }
            const text = await resp.text();
            const textKey =
                `odoo-txt-${action.id || action.report_name}-${Date.now()}-` +
                Math.random().toString(36).slice(2, 10);
            await rpc("/native_print/job_status", {
                idempotency_key: textKey,
                state: "queued",
                device: rule.printer_pattern,
                report_name: action.report_name,
                profile_id: rule.profile_id,
                pages: 1,
                byte_size: text.length,
            });
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
                        profileId: rule.profile_id,
                        byteSize: text.length,
                        pages: 1,
                    },
                    env,
                );
            } catch (error) {
                await rpc("/native_print/job_status", {
                    idempotency_key: textKey,
                    state: "failed",
                    error_code: "BRIDGE_ERROR",
                    device: rule.printer_pattern,
                });
                notifyUser(env, `Native Print error: ${error.message || error}`, "danger");
                throw error;
            }
            await handleCloseOnReportDownload(action, options, env);
            return true;
        }

        const url = buildReportUrl(action, "pdf", env);
        const response = await fetch(url, { credentials: "same-origin" });
        if (!response.ok) {
            throw new Error(`Native Print: report fetch failed (${response.status})`);
        }
        const bytes = Array.from(new Uint8Array(await response.arrayBuffer()));
        const idempotencyKey =
            `odoo-${action.id || action.report_name}-${Date.now()}-` +
            Math.random().toString(36).slice(2, 10);
        await rpc("/native_print/job_status", {
            idempotency_key: idempotencyKey,
            state: "queued",
            device: rule.printer_pattern,
            report_name: action.report_name,
            profile_id: rule.profile_id,
            byte_size: bytes.length,
        });
        try {
            await bridge.printPdf({
                idempotencyKey,
                reportName: action.report_name,
                printerPattern: rule.printer_pattern,
                mode: rule.mode,
                paper: rule.paper,
                copies: rule.copies,
                duplex: !!rule.duplex,
                color: rule.color !== false,
                tray: rule.tray || null,
                scalePolicy: rule.scale_policy,
                url,
                bytes,
            });
            watchJobToCompletion(
                bridge,
                idempotencyKey,
                {
                    reportName: action.report_name,
                    printerPattern: rule.printer_pattern,
                    profileId: rule.profile_id,
                    byteSize: bytes.length,
                    pages: 1,
                },
                env,
            );
        } catch (error) {
            await rpc("/native_print/job_status", {
                idempotency_key: idempotencyKey,
                state: "failed",
                error_code: "BRIDGE_ERROR",
                device: rule.printer_pattern,
            });
            notifyUser(env, `Native Print error: ${error.message || error}`, "danger");
            throw error;
        }
        await handleCloseOnReportDownload(action, options, env);
        return true; // Skip Odoo's default download.
    },
);

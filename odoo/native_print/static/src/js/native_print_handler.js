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
 *   with the user's own session, hand the raw bytes to the bridge, report
 *   the job state back, and return true so Odoo skips its download.
 * - If the server says there is no rule for this report, we also return
 *   undefined: native printing is opt-in per report, by configuration.
 */

function nativePrintBridge() {
    return window.__NATIVE_PRINT__ || null;
}

function buildReportUrl(action) {
    const ids = (action.context && action.context.active_ids) || action.ids || [];
    const docids = Array.isArray(ids) ? ids.join(",") : String(ids);
    return `/report/pdf/${action.report_name}/${docids}`;
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
        if (action.report_type === "qweb-text") {
            // Kitchen tickets and simple text labels: text mode ESC/POS.
            // (Arabic text never uses this path; it prints via PDF/raster.)
            const ids = (action.context && action.context.active_ids) || action.ids || [];
            const docids = Array.isArray(ids) ? ids.join(",") : String(ids);
            const resp = await fetch(`/report/text/${action.report_name}/${docids}`, {
                credentials: "same-origin",
            });
            if (!resp.ok) {
                throw new Error(`Native Print: text report fetch failed (${resp.status})`);
            }
            const text = await resp.text();
            const textKey =
                `odoo-txt-${action.id || action.report_name}-${Date.now()}-` +
                Math.random().toString(36).slice(2, 10);
            await bridge.printText({
                idempotencyKey: textKey,
                reportName: action.report_name,
                printerPattern: resolved.rule.printer_pattern,
                text,
                cut: true,
            });
            return true;
        }
        const url = buildReportUrl(action);
        const response = await fetch(url, { credentials: "same-origin" });
        if (!response.ok) {
            throw new Error(`Native Print: report fetch failed (${response.status})`);
        }
        const bytes = new Uint8Array(await response.arrayBuffer());
        const idempotencyKey =
            `odoo-${action.id || action.report_name}-${Date.now()}-` +
            Math.random().toString(36).slice(2, 10);
        await rpc("/native_print/job_status", {
            idempotency_key: idempotencyKey,
            state: "queued",
            report_name: action.report_name,
            byte_size: bytes.length,
        });
        try {
            // The bridge returns once the job is durably queued in the app.
            // Later states (rendering/spooling/printing/done) are the app's
            // own queue states; the job log row created above is updated by
            // the app through this same endpoint when the app holds a
            // session, and is always visible in the app's diagnostics.
            await bridge.printPdf({
                idempotencyKey,
                reportName: action.report_name,
                printerPattern: resolved.rule.printer_pattern,
                mode: resolved.rule.mode,
                paper: resolved.rule.paper,
                copies: resolved.rule.copies,
                scalePolicy: resolved.rule.scale_policy,
                url,
                bytes,
            });
        } catch (error) {
            await rpc("/native_print/job_status", {
                idempotency_key: idempotencyKey,
                state: "failed",
                error_code: "BRIDGE_ERROR",
            });
            throw error;
        }
        if (action.close_on_report_download && options && options.onClose) {
            options.onClose();
        }
        return true; // Skip Odoo's default download.
    },
);

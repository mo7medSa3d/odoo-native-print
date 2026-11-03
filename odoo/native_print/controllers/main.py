from odoo import http
from odoo.http import request

JOB_STATES = {
    "queued", "rendering", "spooling", "printing",
    "done", "failed", "canceled", "dead_letter",
}

TERMINAL_STATES = {"done", "failed", "canceled", "dead_letter"}


def _safe_int(val, default=None):
    if val is None or val is False or val == "":
        return default
    try:
        return int(val)
    except (ValueError, TypeError):
        return default


class NativePrintController(http.Controller):
    """Job log endpoints for the Native Print app (API v1).

    Printers are deliberately NOT modelled on the server: the desktop
    app on the user's own PC is the only printer authority, the Odoo
    page discovers them live through the bridge and prints to the one
    the user selected. This controller only keeps the job log — the
    desktop reports job outcomes here by idempotency key, scoped to
    the caller's company and user.
    """

    @http.route("/native_print/job_status", type="jsonrpc", auth="user",
                methods=["POST"], csrf=False)
    def job_status(self, idempotency_key=None, state=None, error_code=None,
                   device=None, report_name=None,
                   pages=None, byte_size=None, attempts=None, **kwargs):
        if not (request.env.user.has_group("native_print.group_native_print_user")
                or request.env.user.has_group("native_print.group_native_print_manager")):
            return {"ok": False, "error": "forbidden"}
        if (
            not idempotency_key
            or not isinstance(idempotency_key, str)
            or len(idempotency_key) > 128
            or state not in JOB_STATES
        ):
            return {"ok": False, "error": "bad_request"}
        company = request.env.company.id
        Job = request.env["native.print.job"].sudo().with_context(
            native_print_status_sync=True)
        job = Job.search([
            ("name", "=", idempotency_key),
            ("company_id", "=", company),
        ], limit=1)
        if job and job.user_id and job.user_id.id != request.env.user.id:
            return {"ok": False, "error": "forbidden"}
        if job and job.state in TERMINAL_STATES:
            # Terminal states are final: a late or retried status post
            # must never resurrect or rewrite a finished job's outcome.
            return {"ok": True, "job_id": job.id, "state": job.state,
                    "ignored": "terminal_state_is_final"}
        vals = {
            "state": state,
            "error_code": str(error_code)[:128] if error_code else False,
            "device": str(device)[:256] if device else (job.device if job else False),
        }
        parsed_attempts = _safe_int(attempts)
        if parsed_attempts is not None and parsed_attempts >= 0:
            vals["attempts"] = parsed_attempts
        parsed_pages = _safe_int(pages)
        if parsed_pages is not None and parsed_pages > 0:
            vals["pages"] = parsed_pages
        if not job:
            report = False
            if report_name and isinstance(report_name, str):
                report = request.env["ir.actions.report"].sudo().search(
                    [("report_name", "=", report_name)], limit=1)
            parsed_bytes = _safe_int(byte_size, 0)
            vals.update({
                "name": idempotency_key,
                "report_id": report.id if report else False,
                "report_name": str(report_name)[:256] if report_name else False,
                "pages": parsed_pages if (parsed_pages and parsed_pages > 0) else 1,
                "byte_size": max(0, parsed_bytes or 0),
                "company_id": company,
                "user_id": request.env.user.id,
            })
            job = Job.create(vals)
        else:
            job.write(vals)
        return {"ok": True, "job_id": job.id, "state": job.state}

    @http.route("/native_print/overview", type="jsonrpc", auth="user",
                methods=["POST"], csrf=False)
    def overview(self, **kwargs):
        """Job counters for the Native Print systray widget."""
        if not (request.env.user.has_group("native_print.group_native_print_user")
                or request.env.user.has_group("native_print.group_native_print_manager")):
            return {"ok": False, "error": "forbidden"}
        Job = request.env["native.print.job"]
        domain = [("company_id", "=", request.env.company.id)]
        return {
            "ok": True,
            "active_jobs": Job.search_count(
                domain + [("state", "in", ["queued", "rendering", "spooling", "printing"])]
            ),
            "done_jobs": Job.search_count(domain + [("state", "=", "done")]),
            "failed_jobs": Job.search_count(
                domain + [("state", "in", ["failed", "dead_letter"])]
            ),
        }

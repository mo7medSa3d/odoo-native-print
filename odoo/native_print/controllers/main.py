from psycopg2 import IntegrityError

from odoo import http
from odoo.http import request

JOB_STATES = {
    "queued", "rendering", "spooling", "printing",
    "done", "failed", "canceled", "dead_letter",
}

TERMINAL_STATES = {"done", "failed", "canceled", "dead_letter"}

# The formal job state machine (mirrors the desktop queue, print-core):
# a job only ever moves FORWARD. Working states progress
# queued -> rendering -> spooling -> printing; forward skips are legal
# (an intermediate status post may have been lost), any non-terminal
# state may end in a terminal state except that a job already printing
# can no longer be canceled (the spooler owns it by then). Backward
# moves and exits from terminal states are rejected: they can only
# come from a stale watcher or a duplicated post, and must never
# regress the recorded outcome.
ALLOWED_TRANSITIONS = {
    "queued": {"rendering", "spooling", "printing",
               "done", "failed", "canceled", "dead_letter"},
    "rendering": {"spooling", "printing",
                  "done", "failed", "canceled", "dead_letter"},
    "spooling": {"printing", "done", "failed", "canceled", "dead_letter"},
    "printing": {"done", "failed", "dead_letter"},
}


SOURCE_TYPES = {"report", "pos_receipt", "test"}

PRINT_MODES = {"gdi", "escpos", "zpl", "tspl", "epl"}


def _safe_source_ids(val):
    """Sanitise a comma-separated record-id list from a status post."""
    if not val or not isinstance(val, str):
        return False
    ids = [part.strip() for part in val.split(",")]
    ids = [part for part in ids if part.isdigit()]
    if not ids:
        return False
    return ",".join(ids[:200])[:1024]


def _safe_model_name(val):
    if not val or not isinstance(val, str):
        return False
    val = val.strip()
    if len(val) > 128 or not all(c.isalnum() or c in "._" for c in val):
        return False
    return val


def _safe_int(val, default=None):
    if val is None or val is False or val == "":
        return default
    try:
        return int(val)
    except (ValueError, TypeError):
        return default


class NativePrintController(http.Controller):
    """Job log endpoints for the Yaseir Print app (API v1).

    Which printer prints is deliberately NOT decided on the server:
    the desktop app on the user's own PC is the only printer
    authority, the Odoo page discovers printers live through the
    bridge and prints to the one the user selected. This controller
    only keeps the job log — the page reports job outcomes here by
    idempotency key, scoped to the caller's company and user — and
    registers each reporting printer in a small picker registry so
    the log's printer field is a selectable Many2one.
    """

    @http.route("/native_print/job_status", type="jsonrpc", auth="user",
                methods=["POST"], csrf=False)
    def job_status(self, idempotency_key=None, state=None, error_code=None,
                   device=None, report_name=None,
                   pages=None, byte_size=None, attempts=None,
                   source_type=None, source_model=None, source_ids=None,
                   copies=None, paper=None, duplex=None, color=None,
                   mode=None, **kwargs):
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
        if job and state != job.state and \
                state not in ALLOWED_TRANSITIONS.get(job.state, set()):
            # Illegal move under the state machine (backward, or
            # canceling a job that is already printing).
            return {"ok": True, "job_id": job.id, "state": job.state,
                    "ignored": "illegal_transition"}
        device_name = str(device)[:256] if device else None
        vals = {
            "state": state,
            "error_code": str(error_code)[:128] if error_code else False,
            "device": device_name or (job.device if job else False),
        }
        if device_name:
            # Keep the printer registry in sync with reality: the
            # first job from a printer registers it, so the log's
            # printer field is a pickable Many2one, never free text
            # the user has to type or memorise.
            Device = request.env["native.print.device"].sudo()
            printer = Device.search([
                ("name", "=", device_name),
                ("company_id", "=", company),
            ], limit=1)
            if not printer:
                # Race-safe registration: two jobs reporting the same
                # new printer at the same moment must still produce
                # ONE registry row (unique name+company) - on a
                # conflict, roll back to the savepoint and re-read
                # the row the other post created.
                try:
                    with Device.env.cr.savepoint():
                        printer = Device.create({
                            "name": device_name,
                            "company_id": company,
                        })
                except IntegrityError:
                    printer = Device.search([
                        ("name", "=", device_name),
                        ("company_id", "=", company),
                    ], limit=1)
            if printer and (not job or not job.device_id):
                vals["device_id"] = printer.id
        parsed_attempts = _safe_int(attempts)
        if parsed_attempts is not None and parsed_attempts >= 0:
            # Attempts are monotonic: a stale post must not lower the
            # recorded attempt count of an existing job.
            if not job or parsed_attempts >= (job.attempts or 0):
                vals["attempts"] = parsed_attempts
        parsed_pages = _safe_int(pages)
        if parsed_pages is not None and parsed_pages > 0:
            vals["pages"] = parsed_pages
        parsed_copies = _safe_int(copies, 1)
        if parsed_copies is None or parsed_copies < 1:
            parsed_copies = 1
        if parsed_copies > 99:
            parsed_copies = 99
        created = False
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
                "source_type": source_type if source_type in SOURCE_TYPES
                               else "report",
                "source_model": _safe_model_name(source_model),
                "source_ids": _safe_source_ids(source_ids),
                "copies": parsed_copies,
                "paper": str(paper)[:64] if isinstance(paper, str)
                         and paper else False,
                "duplex": bool(duplex),
                "color": bool(color) if color is not None else True,
                "mode": mode if mode in PRINT_MODES else False,
                "pages": parsed_pages if (parsed_pages and parsed_pages > 0) else 1,
                "byte_size": max(0, parsed_bytes or 0),
                "company_id": company,
                "user_id": request.env.user.id,
            })
            # Race-safe creation, mirroring device registration
            # above: two concurrent first posts for one key+company
            # must produce ONE job row. The loser of the unique
            # constraint re-reads the winner's row and falls into the
            # update path below instead of failing the post with a
            # 500 (its state/device/attempts updates still apply).
            try:
                with Job.env.cr.savepoint():
                    job = Job.create(vals)
                created = True
            except IntegrityError:
                job = Job.search([
                    ("name", "=", idempotency_key),
                    ("company_id", "=", company),
                ], limit=1)
        if job and not created:
            # A later post may carry the source reference the creating
            # post missed (or vice versa): fill blanks, never overwrite.
            if not job.source_ids:
                parsed_ids = _safe_source_ids(source_ids)
                if parsed_ids:
                    vals["source_ids"] = parsed_ids
            if not job.source_model:
                parsed_model = _safe_model_name(source_model)
                if parsed_model:
                    vals["source_model"] = parsed_model
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

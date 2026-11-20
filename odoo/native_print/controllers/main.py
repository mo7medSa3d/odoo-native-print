from psycopg2 import IntegrityError

from odoo import fields, http
from odoo.http import request

JOB_STATES = {
    "queued", "rendering", "spooling", "printing",
    "done", "failed", "canceled", "dead_letter",
}

# Only `done` and `canceled` are final; `failed` and `dead_letter`
# can re-enter `queued` when retried by the desktop worker or operator.
TERMINAL_STATES = {"done", "canceled"}

# The formal job state machine (mirrors the desktop queue, print-core):
# a job only ever moves FORWARD. Working states progress
# queued -> rendering -> spooling -> printing; forward skips are legal
# (an intermediate status post may have been lost), any non-terminal
# state may end in a terminal state except that a job already printing
# can no longer be canceled (the spooler owns it by then). Backward
# moves and exits from `done`/`canceled` are rejected: they can only
# come from a stale watcher or a duplicated post, and must never
# regress the recorded outcome. `failed` and `dead_letter` re-enter
# at `queued` on retry (forward skips tolerated for a lost `queued`
# post); they are NOT exits from a final state.
ALLOWED_TRANSITIONS = {
    "queued": {"rendering", "spooling", "printing",
               "done", "failed", "canceled", "dead_letter"},
    "rendering": {"spooling", "printing",
                  "done", "failed", "canceled", "dead_letter"},
    "spooling": {"printing", "done", "failed", "canceled", "dead_letter"},
    "printing": {"done", "failed", "dead_letter"},
    "failed": {"queued", "rendering", "spooling", "printing",
               "done", "canceled", "dead_letter"},
    "dead_letter": {"queued", "rendering", "spooling", "printing",
                    "done", "failed", "canceled"},
}


SOURCE_TYPES = {"report", "pos_receipt", "test"}

PRINT_MODES = {"gdi", "escpos", "zpl", "tspl", "epl"}


def _safe_source_ids(val):
    """Sanitise a comma-separated record-id list from a status post."""
    if not val or not isinstance(val, str):
        return False
    ids = []
    total_len = 0
    for raw in val.split(","):
        part = raw.strip()
        if not (part.isascii() and part.isdigit()):
            continue
        rec_id = int(part)
        if not (1 <= rec_id <= 2_147_483_647):
            continue
        token = str(rec_id)
        added_len = len(token) + (1 if ids else 0)
        if len(ids) >= 200 or total_len + added_len > 1024:
            break
        ids.append(token)
        total_len += added_len
    if not ids:
        return False
    return ",".join(ids)


def _safe_model_name(val):
    if not val or not isinstance(val, str):
        return False
    val = val.strip()
    if (
        not val
        or len(val) > 128
        or not val.isascii()
        or not all(c.isalnum() or c in "._" for c in val)
        or val[0] in "._"
        or val[-1] in "._"
        or ".." in val
    ):
        return False
    return val


def _safe_int(val, default=None):
    if val is None or isinstance(val, bool) or val == "":
        return default
    try:
        # Clamp to PostgreSQL int4 max (desktop queue allows up to 50 MB).
        return max(0, min(int(val), 2_147_483_647))
    except (ValueError, TypeError):
        return default


_STATUS_LOCK_FIELDS = (
    "state", "user_id", "company_id", "attempts", "device", "device_id",
    "source_ids", "source_model", "report_name", "report_id",
    "duplex", "duplex_configured",
)

def _lock_status_job(job):
    """Serialize status updates and refresh the ORM cache after the lock.

    `_table` is the model's fixed ORM metadata (not request input). The
    lock closes the search/check/write race: after waiting for another
    updater, the next request must validate against its committed state.
    """
    if not job:
        return job
    job.flush_recordset(_STATUS_LOCK_FIELDS)
    job.env.cr.execute(
        f'SELECT id FROM "{job._table}" WHERE id = %s FOR UPDATE',
        [job.id],
    )
    if not job.env.cr.fetchone():
        return job.browse()
    job.invalidate_recordset(_STATUS_LOCK_FIELDS)
    return job


def _has_print_access():
    user = request.env.user
    return (
        user.has_group("native_print.group_native_print_user")
        or user.has_group("native_print.group_native_print_manager")
    )


def _get_or_create_device(device_name, company_id, vals=None):
    Device = request.env["native.print.device"].sudo()
    printer = Device.search([
        ("name", "=", device_name),
        ("company_id", "=", company_id),
    ], limit=1)
    if not printer:
        create_vals = dict(vals or {}, name=device_name, company_id=company_id)
        try:
            with Device.env.cr.savepoint():
                printer = Device.create(create_vals)
        except IntegrityError:
            printer = Device.search([
                ("name", "=", device_name),
                ("company_id", "=", company_id),
            ], limit=1)
            if printer and vals:
                printer.write(vals)
    elif vals:
        printer.write(vals)
    return printer


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
        if not _has_print_access():
            return {"ok": False, "error": "forbidden"}
        if not isinstance(idempotency_key, str):
            return {"ok": False, "error": "bad_request"}
        idempotency_key = idempotency_key.strip()
        if not idempotency_key or len(idempotency_key) > 128 or state not in JOB_STATES:
            return {"ok": False, "error": "bad_request"}
        company = request.env.company.id
        Job = request.env["native.print.job"].sudo().with_context(
            native_print_status_sync=True)
        job = Job.search([
            ("name", "=", idempotency_key),
            ("company_id", "=", company),
        ], limit=1)
        job = _lock_status_job(job)
        if job and job.user_id.id != request.env.user.id:
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
        device_name = (device.strip()[:256] or None) if isinstance(device, str) else None
        clean_report_name = (
            report_name.strip()[:256]
            if isinstance(report_name, str) and report_name.strip()
            else False
        )
        vals = {"state": state}
        if isinstance(error_code, str) and error_code.strip():
            vals["error_code"] = error_code.strip()[:128]
        elif not job or state != job.state or error_code == "":
            # Clear stale error_code on state transitions or explicit clear,
            # but preserve it on same-state metadata reposts where error_code is omitted.
            vals["error_code"] = False
        if device_name:
            vals["device"] = device_name
        # `duplex=None` means leave the driver default in force. Only a
        # JSON boolean is an explicit choice (including False/simplex).
        if isinstance(duplex, bool) and (not job or not job.duplex_configured):
            vals["duplex"] = duplex
            vals["duplex_configured"] = True
        if device_name:
            # Keep the printer registry in sync with reality: the
            # first job from a printer registers it, so the log's
            # printer field is a pickable Many2one, never free text
            # the user has to type or memorise.
            # Race-safe registration: two jobs reporting the same new
            # printer at the same moment must still produce ONE row.
            printer = _get_or_create_device(device_name, company)
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
            if clean_report_name:
                report = request.env["ir.actions.report"].sudo().search(
                    [("report_name", "=", clean_report_name)], limit=1)
            parsed_bytes = _safe_int(byte_size, 0)
            create_vals = dict(vals, **{
                "name": idempotency_key,
                "report_id": report.id if report else False,
                "report_name": clean_report_name,
                "source_type": source_type if source_type in SOURCE_TYPES
                               else "report",
                "source_model": _safe_model_name(source_model),
                "source_ids": _safe_source_ids(source_ids),
                "copies": parsed_copies,
                "paper": paper.strip()[:64] if isinstance(paper, str)
                         and paper.strip() else False,
                "duplex": bool(duplex) if isinstance(duplex, bool) else False,
                "duplex_configured": isinstance(duplex, bool),
                "color": color if isinstance(color, bool) else True,
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
                    job = Job.create(create_vals)
                created = True
            except IntegrityError:
                job = Job.search([
                    ("name", "=", idempotency_key),
                    ("company_id", "=", company),
                ], limit=1)
                job = _lock_status_job(job)
                if not job:
                    # Winning transaction not yet committed (READ COMMITTED): ask client to retry.
                    return {"ok": False, "error": "retry"}
                # Re-apply ownership, terminal-state, transition, and attempt guards
                # against the winner's row before updating.
                if job.user_id.id != request.env.user.id:
                    return {"ok": False, "error": "forbidden"}
                if job.state in TERMINAL_STATES:
                    return {"ok": True, "job_id": job.id, "state": job.state,
                            "ignored": "terminal_state_is_final"}
                if state != job.state and \
                        state not in ALLOWED_TRANSITIONS.get(job.state, set()):
                    return {"ok": True, "job_id": job.id, "state": job.state,
                            "ignored": "illegal_transition"}
                if "attempts" in vals and vals["attempts"] < (job.attempts or 0):
                    del vals["attempts"]
                if job.duplex_configured:
                    vals.pop("duplex", None)
                    vals.pop("duplex_configured", None)
                if job.device_id:
                    vals.pop("device_id", None)
                if job.state == state and error_code is None:
                    vals.pop("error_code", None)
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
            if not job.report_name and clean_report_name:
                vals["report_name"] = clean_report_name
            if not job.report_id and clean_report_name:
                report = request.env["ir.actions.report"].sudo().search(
                    [("report_name", "=", clean_report_name)], limit=1)
                if report:
                    vals["report_id"] = report.id
            job.write(vals)
        return {"ok": True, "job_id": job.id, "state": job.state}

    @http.route("/native_print/sync_device", type="jsonrpc", auth="user",
                methods=["POST"], csrf=False)
    def sync_device(self, name=None, protocol=None, printer_class=None,
                    forms=None, trays=None, **kwargs):
        """Attach a driver capability snapshot to a registry device.

        The desktop app reports the selected printer's driver capabilities
        (paper forms and trays) for read-only display in Odoo.
        Upsert on the same (name, company) unique key the job sync
        uses: the same physical printer can never register twice.
        """
        if not _has_print_access():
            return {"ok": False, "error": "forbidden"}
        if not name or not isinstance(name, str) or not name.strip():
            return {"ok": False, "error": "bad_request"}
        device_name = name.strip()[:256]

        def _clean_list(val):
            if not isinstance(val, list):
                return []
            return [" ".join(item.split())[:160] for item in val[:80]
                    if isinstance(item, str) and item.strip()]

        forms_list = _clean_list(forms)
        trays_list = _clean_list(trays)
        vals = {
            "protocol": protocol.strip()[:32] if isinstance(protocol, str) and protocol.strip() else False,
            "printer_class": printer_class.strip()[:32] if isinstance(printer_class, str) and printer_class.strip() else False,
            "forms_summary": "\n".join(forms_list) or False,
            "trays_summary": "\n".join(trays_list) or False,
            "caps_synced_at": fields.Datetime.now(),
        }
        company = request.env.company.id
        printer = _get_or_create_device(device_name, company, vals)
        if not printer:
            return {"ok": False, "error": "registration_failed"}
        return {"ok": True, "device_id": printer.id}

    @http.route("/native_print/overview", type="jsonrpc", auth="user",
                methods=["POST"], csrf=False)
    def overview(self, **kwargs):
        """Job counters for the Native Print systray widget."""
        if not _has_print_access():
            return {"ok": False, "error": "forbidden"}
        # Counters are company-wide across roles; sudo + explicit company
        # domain bypasses the per-user record rule while keeping company scope.
        Job = request.env["native.print.job"].sudo()
        domain = [("company_id", "in", request.env.companies.ids)]
        counts = dict(
            Job._read_group(domain, groupby=["state"], aggregates=["__count"])
        )
        return {
            "ok": True,
            "active_jobs": sum(
                counts.get(s, 0)
                for s in ("queued", "rendering", "spooling", "printing")
            ),
            "done_jobs": counts.get("done", 0),
            "failed_jobs": (
                counts.get("failed", 0) + counts.get("dead_letter", 0)
            ),
        }

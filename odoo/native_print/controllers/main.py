from odoo import http
from odoo.http import request

JOB_STATES = {
    "queued", "rendering", "spooling", "printing",
    "done", "failed", "canceled", "dead_letter",
}


def _safe_int(val, default=None):
    if val is None or val is False or val == "":
        return default
    try:
        return int(val)
    except (ValueError, TypeError):
        return default


class NativePrintController(http.Controller):
    """Status and rule resolution endpoints for the Native Print app (API v1).

    The app reports job state changes here using the idempotency key.
    Authentication is the user's own session; the server remains the
    authority: unknown keys create a log row for the caller's company
    only, and a caller can never touch another company's jobs (the
    record rule on native.print.job also enforces this).
    """

    @http.route("/native_print/job_status", type="jsonrpc", auth="user",
                methods=["POST"], csrf=False)
    def job_status(self, idempotency_key=None, state=None, error_code=None,
                   device=None, report_name=None, profile_id=None,
                   pages=None, byte_size=None, attempts=None, **kwargs):
        if (
            not idempotency_key
            or not isinstance(idempotency_key, str)
            or len(idempotency_key) > 128
            or state not in JOB_STATES
        ):
            return {"ok": False, "error": "bad_request"}
        company = request.env.company.id
        Job = request.env["native.print.job"].sudo()
        job = Job.search([
            ("name", "=", idempotency_key),
            ("company_id", "=", company),
        ], limit=1)
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
            parsed_profile_id = _safe_int(profile_id)
            valid_profile_id = False
            if parsed_profile_id:
                prof = request.env["native.print.profile"].sudo().search([
                    ("id", "=", parsed_profile_id),
                    ("company_id", "=", company),
                ], limit=1)
                if prof:
                    valid_profile_id = prof.id
            parsed_bytes = _safe_int(byte_size, 0)
            vals.update({
                "name": idempotency_key,
                "report_id": report.id if report else False,
                "report_name": str(report_name)[:256] if report_name else False,
                "profile_id": valid_profile_id,
                "pages": parsed_pages if (parsed_pages and parsed_pages > 0) else 1,
                "byte_size": max(0, parsed_bytes or 0),
                "company_id": company,
                "user_id": request.env.user.id,
            })
            job = Job.create(vals)
        else:
            job.write(vals)
        return {"ok": True, "job_id": job.id, "state": job.state}

    def _rule_payload(self, rule):
        p = rule.profile_id
        return {"ok": True, "rule": {
            "rule_id": rule.id,
            "profile_id": p.id,
            "profile": p.name,
            "printer_pattern": p.printer_pattern,
            "mode": p.mode,
            "paper": p.paper,
            "copies": p.copies,
            "tray": p.tray,
            "duplex": p.duplex,
            "color": p.color,
            "scale_policy": p.scale_policy,
            "recovery_mode": p.recovery_mode,
        }}

    def _first_matching_rule(self, domain):
        rules = request.env["native.print.rule"].search(domain)
        user = request.env.user
        # Odoo 19 renamed groups_id -> group_ids, with all_group_ids including implied groups
        groups = getattr(user, "all_group_ids", None)
        if groups is None:
            groups = getattr(user, "group_ids", getattr(user, "groups_id", None))
        user_groups = set(groups.ids) if groups else set()
        for rule in rules:  # model _order = priority desc, id
            if not rule.profile_id or not rule.profile_id.active:
                continue
            if rule.group_id and rule.group_id.id not in user_groups:
                continue
            return rule
        return None

    @http.route("/native_print/rules", type="jsonrpc", auth="user",
                methods=["POST"], csrf=False)
    def rules_for_report(self, report_name=None, **kwargs):
        """Resolve the printing rule for a report, server-side.

        Precedence:
        1. Exact match on `report_id` (highest priority active rule for user's group/company).
        2. Exact match on `report_name` (e.g. `point_of_sale.receipt` or report technical name).
        3. Wildcard catch-all rule (`report_id = False` and `report_name in (False, '', '*')`).
        """
        if not report_name or not isinstance(report_name, str):
            return {"ok": False, "error": "bad_request"}
        company = request.env.company.id
        report = request.env["ir.actions.report"].search(
            [("report_name", "=", report_name)], limit=1)
        if report:
            rule = self._first_matching_rule([
                ("report_id", "=", report.id),
                ("company_id", "=", company),
            ])
            if rule:
                return self._rule_payload(rule)
        rule = self._first_matching_rule([
            ("report_name", "=", report_name),
            ("company_id", "=", company),
        ])
        if rule:
            return self._rule_payload(rule)
        # Catch-all wildcard rule (mirrors print-core::routing::resolve_rule).
        catch_all = self._first_matching_rule([
            ("report_id", "=", False),
            ("report_name", "in", [False, "", "*"]),
            ("company_id", "=", company),
        ])
        if catch_all:
            return self._rule_payload(catch_all)
        return {"ok": True, "rule": None}

    @http.route("/native_print/overview", type="jsonrpc", auth="user",
                methods=["POST"], csrf=False)
    def overview(self, **kwargs):
        """Return server-side counts for the Native Print Systray widget."""
        company = request.env.company.id
        Profile = request.env["native.print.profile"]
        Rule = request.env["native.print.rule"]
        Job = request.env["native.print.job"]
        domain = [("company_id", "=", company)]
        return {
            "ok": True,
            "profiles_count": Profile.search_count(domain),
            "rules_count": Rule.search_count(domain),
            "active_jobs": Job.search_count(
                domain + [("state", "in", ["queued", "rendering", "spooling", "printing"])]
            ),
            "done_jobs": Job.search_count(domain + [("state", "=", "done")]),
            "failed_jobs": Job.search_count(
                domain + [("state", "in", ["failed", "dead_letter"])]
            ),
        }

from odoo import http
from odoo.http import request

JOB_STATES = {
    "queued", "rendering", "spooling", "printing",
    "done", "failed", "canceled", "dead_letter",
}


class NativePrintController(http.Controller):
    """Status loop for the native app (API contract v1).

    The app reports job state changes here using the idempotency key.
    Authentication is the user's own session; the server remains the
    authority: unknown keys create a log row for the caller's company
    only, and a caller can never touch another company's jobs (the
    record rule on native.print.job also enforces this).
    """

    @http.route("/native_print/job_status", type="jsonrpc", auth="user",
                methods=["POST"], csrf=False)
    def job_status(self, idempotency_key=None, state=None, error_code=None,
                   device=None, report_name=None, pages=None, byte_size=None,
                   **kwargs):
        if not idempotency_key or state not in JOB_STATES:
            return {"ok": False, "error": "bad_request"}
        Job = request.env["native.print.job"].sudo()
        job = Job.search([("name", "=", idempotency_key)], limit=1)
        vals = {
            "state": state,
            "error_code": error_code or False,
            "device": device or (job.device if job else False),
        }
        if not job:
            report = False
            if report_name:
                report = request.env["ir.actions.report"].sudo().search(
                    [("report_name", "=", report_name)], limit=1)
            vals.update({
                "name": idempotency_key,
                "report_id": report.id if report else False,
                "pages": pages or 0,
                "byte_size": byte_size or 0,
            })
            job = Job.create(vals)
        else:
            job.write(vals)
        return {"ok": True, "job_id": job.id, "state": job.state}

    def _rule_payload(self, rule):
        p = rule.profile_id
        return {"ok": True, "rule": {
            "profile": p.name,
            "printer_pattern": p.printer_pattern,
            "mode": p.mode,
            "paper": p.paper,
            "copies": p.copies,
            "scale_policy": p.scale_policy,
            "recovery_mode": p.recovery_mode,
        }}

    def _first_matching_rule(self, domain):
        rules = request.env["native.print.rule"].search(domain)
        user_groups = set(request.env.user.groups_id.ids)
        for rule in rules:  # model _order = priority desc
            if rule.group_id and rule.group_id.id not in user_groups:
                continue
            return rule
        return None

    @http.route("/native_print/rules", type="jsonrpc", auth="user",
                methods=["POST"], csrf=False)
    def rules_for_report(self, report_name=None, **kwargs):
        """Resolve the printing rule for a report, server-side.

        Returns the winning rule's profile (highest priority, caller's
        company) or an empty result meaning 'no native printing for this
        report' — the handler then falls back to normal Odoo behaviour.

        Two kinds of sources resolve here:
        - ir.actions.report reports, matched by the rule's report_id;
        - non-report sources (e.g. POS receipts, source name
          'point_of_sale.receipt'), matched by the rule's report_name
          field, because no ir.actions.report record exists for them.
        """
        if not report_name:
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
        return {"ok": True, "rule": None}

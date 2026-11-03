from odoo import fields, models


class NativePrintJob(models.Model):
    _name = "native.print.job"
    _description = "Native Print: job log"
    _order = "create_date desc, id desc"

    name = fields.Char(
        required=True,
        index=True,
        string="Idempotency key",
    )
    state = fields.Selection(
        [
            ("queued", "Queued"),
            ("rendering", "Rendering"),
            ("spooling", "Spooling"),
            ("printing", "Printing"),
            ("done", "Done"),
            ("failed", "Failed"),
            ("canceled", "Canceled"),
            ("dead_letter", "Dead letter"),
        ],
        required=True,
        default="queued",
        index=True,
    )
    error_code = fields.Char(string="Error Code")
    device = fields.Char(string="Printer / Device")
    user_id = fields.Many2one(
        "res.users",
        default=lambda s: s.env.user,
        string="User",
    )
    report_id = fields.Many2one(
        "ir.actions.report",
        string="Report",
    )
    report_name = fields.Char(
        string="Source Name",
        index=True,
    )
    profile_id = fields.Many2one(
        "native.print.profile",
        string="Printer Profile",
    )
    pages = fields.Integer(default=1, string="Pages")
    byte_size = fields.Integer(string="Bytes")
    attempts = fields.Integer(default=1, string="Attempts")
    company_id = fields.Many2one(
        "res.company",
        required=True,
        default=lambda s: s.env.company,
        string="Company",
    )

    def action_retry_job(self):
        for rec in self:
            if rec.state in ("failed", "dead_letter", "canceled"):
                rec.write({
                    "state": "queued",
                    "error_code": False,
                    "attempts": (rec.attempts or 0) + 1,
                })
        return True

    def action_cancel_job(self):
        for rec in self:
            if rec.state in ("queued", "rendering", "spooling"):
                rec.write({
                    "state": "canceled",
                    "error_code": "CANCELED_BY_USER",
                })
        return True

    def action_mark_done(self):
        for rec in self:
            rec.write({
                "state": "done",
                "error_code": False,
            })
        return True

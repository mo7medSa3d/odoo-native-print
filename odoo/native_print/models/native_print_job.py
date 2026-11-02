from odoo import fields, models


class NativePrintJob(models.Model):
    _name = "native.print.job"
    _description = "Native Print: job log"
    _order = "create_date desc"

    name = fields.Char(required=True, index=True, string="Idempotency key")
    state = fields.Selection(
        [("queued", "Queued"), ("rendering", "Rendering"),
         ("spooling", "Spooling"), ("printing", "Printing"),
         ("done", "Done"), ("failed", "Failed"),
         ("canceled", "Canceled"), ("dead_letter", "Dead letter")],
        required=True, default="queued", index=True,
    )
    error_code = fields.Char()
    device = fields.Char()
    user_id = fields.Many2one("res.users", default=lambda s: s.env.user)
    report_id = fields.Many2one("ir.actions.report")
    profile_id = fields.Many2one("native.print.profile")
    pages = fields.Integer()
    byte_size = fields.Integer(string="Bytes")
    company_id = fields.Many2one(
        "res.company", required=True, default=lambda s: s.env.company)

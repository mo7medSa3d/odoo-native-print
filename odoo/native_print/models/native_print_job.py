from odoo import _, fields, models
from odoo.exceptions import UserError


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
    pages = fields.Integer(default=1, string="Pages")
    byte_size = fields.Integer(string="Bytes")
    attempts = fields.Integer(default=1, string="Attempts")
    company_id = fields.Many2one(
        "res.company",
        required=True,
        default=lambda s: s.env.company,
        string="Company",
    )

    _idempotency_key_company_uniq = models.Constraint(
        "unique(name, company_id)",
        "A print job with this idempotency key already exists for this company.",
    )

    def write(self, vals):
        # Job states are facts reported by the desktop app's print worker
        # through /native_print/job_status. Changing them from the Odoo UI
        # would display outcomes that never happened (a "done" job that
        # never printed, a "retry" nothing reprints), so state transitions
        # are accepted only from the status-sync path.
        if "state" in vals and not self.env.context.get(
            "native_print_status_sync"
        ):
            for rec in self:
                if rec.state != vals["state"]:
                    raise UserError(_(
                        "Print job status is reported by the desktop app "
                        "and cannot be changed manually. To print again, "
                        "print the document again from Odoo."
                    ))
        return super().write(vals)

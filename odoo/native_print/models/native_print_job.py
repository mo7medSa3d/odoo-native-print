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
    # The printer as a pickable registry entry (auto-synced from real
    # jobs - see native.print.device). `device` below keeps the raw
    # name snapshot exactly as the desktop reported it.
    device_id = fields.Many2one(
        "native.print.device",
        string="Printer",
        index=True,
        ondelete="set null",
    )
    device = fields.Char(string="Printer (as reported)")
    source_type = fields.Selection(
        [
            ("report", "Report"),
            ("pos_receipt", "POS Receipt"),
            ("test", "Test Print"),
        ],
        default="report",
        required=True,
        index=True,
        string="Source Type",
    )
    # What the report was printed for, so the job can be reprinted:
    # the source model and the record ids captured at print time.
    source_model = fields.Char(string="Source Model", index=True)
    source_ids = fields.Char(string="Source Records")
    # The print settings the job actually ran with, captured at print
    # time so a reprint comes prefilled with the same choices instead
    # of whatever happens to be selected today.
    copies = fields.Integer(default=1, string="Copies")
    paper = fields.Char(string="Paper")
    duplex = fields.Boolean(string="Duplex")
    color = fields.Boolean(default=True, string="Color")
    mode = fields.Char(string="Print Mode")
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

    def action_reprint(self):
        """Re-print this job's document.

        Returns the original report action for the original records,
        so the report is rendered again and - inside the desktop
        app - intercepted and printed exactly like the first time.
        (A fresh render of the document as it stands now, which is
        what "print again" means everywhere in Odoo; the log stores
        no payload copies by design.)
        """
        self.ensure_one()
        ids = []
        if self.source_ids:
            ids = [int(x) for x in self.source_ids.split(",")
                   if x.strip().isdigit()]
        if not self.report_id or not ids:
            raise UserError(_(
                "This job cannot be reprinted: the original report "
                "or its records were not recorded for it (for "
                "example, POS receipts are reprinted from the POS)."
            ))
        settings = {}
        if self.copies:
            settings["copies"] = self.copies
        if self.paper:
            settings["paper"] = self.paper
        settings["duplex"] = bool(self.duplex)
        settings["color"] = bool(self.color)
        printer_name = (self.device_id.name if self.device_id
                        else self.device)
        return {
            "type": "ir.actions.report",
            "report_name": self.report_id.report_name,
            "report_type": self.report_id.report_type,
            "ids": ids,
            "context": {
                "active_ids": ids,
                "active_id": ids[0],
                "active_model": self.source_model or self.report_id.model,
                # Prefill for the print handler: reprint on the job's
                # own printer (when it is connected to this PC) with
                # the job's own settings, not today's selections.
                "native_print_prefill": {
                    "printer": printer_name or None,
                    "settings": settings,
                },
            },
        }

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

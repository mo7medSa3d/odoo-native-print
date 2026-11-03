from odoo import api, fields, models


class NativePrintProfile(models.Model):
    _name = "native.print.profile"
    _description = "Native Print: printer profile"
    _order = "name, id"

    name = fields.Char(required=True, string="Profile Name")
    active = fields.Boolean(default=True)
    printer_pattern = fields.Char(
        required=True,
        string="Printer Name / Pattern",
        help="Exact Windows printer name, wildcard pattern (e.g. HP*, *Thermal*), "
             "or direct network URI (tcp://192.168.1.200:9100 or ipp://192.168.1.50:631/ipp/print).",
    )
    mode = fields.Selection(
        [
            ("gdi", "Office (PDF/GDI)"),
            ("escpos", "Thermal ESC/POS"),
            ("zpl", "Label ZPL"),
            ("tspl", "Label TSPL"),
            ("epl", "Label EPL2"),
            ("ipp", "IPP"),
        ],
        required=True,
        default="gdi",
        string="Print Mode",
    )
    paper = fields.Char(
        string="Paper Format",
        help="Requested paper, e.g. A4, A5, Letter, 80mm, 58mm, 100x150mm.",
    )
    copies = fields.Integer(default=1, string="Default Copies")
    tray = fields.Char(string="Paper Tray / Bin")
    duplex = fields.Boolean(default=False, string="Duplex (Two-Sided)")
    color = fields.Boolean(default=True, string="Color Printing")
    scale_policy = fields.Selection(
        [
            ("strict", "Strict (fail on wrong size)"),
            ("custom_allowed", "Custom size allowed"),
            ("nearest_larger_centered", "Nearest larger, centred, no scale"),
            ("nearest_larger_fit", "Nearest larger, fit"),
        ],
        required=True,
        default="custom_allowed",
        string="Paper Scale Policy",
    )
    recovery_mode = fields.Selection(
        [
            ("safe_no_reprint", "Safe: never auto-reprint an ambiguous job"),
            ("auto_reprint", "Auto-reprint after a crash"),
        ],
        required=True,
        default="safe_no_reprint",
        string="Crash Recovery Policy",
    )
    notes = fields.Text(
        string="Hardware Quirks & Notes",
        help="Optional operator notes, driver version, or physical offset reminders.",
    )
    company_id = fields.Many2one(
        "res.company",
        required=True,
        default=lambda s: s.env.company,
        string="Company",
    )
    rule_count = fields.Integer(
        compute="_compute_counts",
        string="Routing Rules",
    )
    job_count = fields.Integer(
        compute="_compute_counts",
        string="Print Jobs",
    )
    failed_job_count = fields.Integer(
        compute="_compute_counts",
        string="Failed Jobs",
    )

    @api.depends("name")
    def _compute_counts(self):
        Rule = self.env["native.print.rule"]
        Job = self.env["native.print.job"]
        for rec in self:
            if not rec.id:
                rec.rule_count = 0
                rec.job_count = 0
                rec.failed_job_count = 0
                continue
            rec.rule_count = Rule.search_count([("profile_id", "=", rec.id)])
            rec.job_count = Job.search_count([("profile_id", "=", rec.id)])
            rec.failed_job_count = Job.search_count([
                ("profile_id", "=", rec.id),
                ("state", "in", ["failed", "dead_letter"]),
            ])

    def action_view_rules(self):
        self.ensure_one()
        return {
            "type": "ir.actions.act_window",
            "name": "Routing Rules",
            "res_model": "native.print.rule",
            "view_mode": "list,form",
            "domain": [("profile_id", "=", self.id)],
            "context": {"default_profile_id": self.id},
        }

    def action_view_jobs(self):
        self.ensure_one()
        return {
            "type": "ir.actions.act_window",
            "name": "Print Jobs",
            "res_model": "native.print.job",
            "view_mode": "list,kanban,form",
            "domain": [("profile_id", "=", self.id)],
            "context": {"default_profile_id": self.id},
        }

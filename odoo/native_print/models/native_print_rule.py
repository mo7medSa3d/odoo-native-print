from odoo import api, fields, models


class NativePrintRule(models.Model):
    _name = "native.print.rule"
    _description = "Native Print: routing rule"
    _order = "priority desc, id"

    name = fields.Char(required=True, string="Rule Name")
    company_id = fields.Many2one(
        "res.company",
        required=True,
        default=lambda s: s.env.company,
        string="Company",
    )
    report_id = fields.Many2one(
        "ir.actions.report",
        string="Odoo Report",
        help="Select a standard Odoo report (qweb-pdf or qweb-text).",
    )
    report_name = fields.Char(
        string="Source name (non-report)",
        help="For print sources that are not ir.actions.report reports. "
             "Use 'point_of_sale.receipt' for POS receipts (the POS "
             "extension resolves its rule by this name). Leave empty for "
             "normal report rules, which match by Report above.",
    )
    group_id = fields.Many2one(
        "res.groups",
        string="Restricted to Group",
        help="Optional: apply this rule only when the user belongs to this security group.",
    )
    profile_id = fields.Many2one(
        "native.print.profile",
        required=True,
        string="Printer Profile",
    )
    mode = fields.Selection(
        related="profile_id.mode",
        string="Print Mode",
        readonly=True,
    )
    printer_pattern = fields.Char(
        related="profile_id.printer_pattern",
        string="Printer Pattern",
        readonly=True,
    )
    target_source_display = fields.Char(
        compute="_compute_target_source_display",
        string="Matched Source",
    )
    priority = fields.Integer(
        default=10,
        help="Higher priority rules win when multiple rules match the same report.",
    )
    active = fields.Boolean(default=True)

    @api.depends("report_id", "report_name")
    def _compute_target_source_display(self):
        for rec in self:
            if rec.report_id:
                rec.target_source_display = (
                    f"{rec.report_id.name} ({rec.report_id.report_name})"
                )
            elif rec.report_name:
                rec.target_source_display = rec.report_name
            else:
                rec.target_source_display = "*"

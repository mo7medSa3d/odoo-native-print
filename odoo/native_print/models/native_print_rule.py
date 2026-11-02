from odoo import fields, models


class NativePrintRule(models.Model):
    _name = "native.print.rule"
    _description = "Native Print: routing rule"
    _order = "priority desc, id"

    name = fields.Char(required=True)
    company_id = fields.Many2one(
        "res.company", required=True, default=lambda s: s.env.company)
    report_id = fields.Many2one("ir.actions.report")
    report_name = fields.Char(
        string="Source name (non-report)",
        help="For print sources that are not ir.actions.report reports. "
             "Use 'point_of_sale.receipt' for POS receipts (the POS "
             "extension resolves its rule by this name). Leave empty for "
             "normal report rules, which match by Report above.")
    group_id = fields.Many2one("res.groups")
    profile_id = fields.Many2one("native.print.profile", required=True)
    priority = fields.Integer(default=10)
    active = fields.Boolean(default=True)

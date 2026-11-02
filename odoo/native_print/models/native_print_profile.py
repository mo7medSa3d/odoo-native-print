from odoo import fields, models


class NativePrintProfile(models.Model):
    _name = "native.print.profile"
    _description = "Native Print: printer profile"

    name = fields.Char(required=True)
    printer_pattern = fields.Char(
        required=True,
        help="Exact printer name or a pattern matched on the client device.",
    )
    mode = fields.Selection(
        [("gdi", "Office (PDF/GDI)"), ("escpos", "Thermal ESC/POS"),
         ("zpl", "Label ZPL"), ("tspl", "Label TSPL"), ("ipp", "IPP")],
        required=True, default="gdi",
    )
    paper = fields.Char(help="Requested paper, e.g. A4, A5, 80mm, 100x150mm.")
    copies = fields.Integer(default=1)
    tray = fields.Char()
    duplex = fields.Boolean(default=False)
    color = fields.Boolean(default=True)
    scale_policy = fields.Selection(
        [("strict", "Strict (fail on wrong size)"),
         ("custom_allowed", "Custom size allowed"),
         ("nearest_larger_centered", "Nearest larger, centred, no scale"),
         ("nearest_larger_fit", "Nearest larger, fit")],
        required=True, default="custom_allowed",
    )
    recovery_mode = fields.Selection(
        [("safe_no_reprint", "Safe: never auto-reprint an ambiguous job"),
         ("auto_reprint", "Auto-reprint after a crash")],
        required=True, default="safe_no_reprint",
    )
    company_id = fields.Many2one(
        "res.company", required=True, default=lambda s: s.env.company)

from odoo import fields, models


class NativePrintDevice(models.Model):
    """Registry of printers that have actually printed, per company.

    This model exists for ONE reason: so a print job's printer is a
    real Many2one the user can pick, filter and group by in the job
    log, instead of a free-text string nobody should have to type or
    memorise. Rows are created automatically by the job-status sync
    the first time a printer produces a job; there is no
    configuration and no manual workflow for it. Managers get one
    small maintenance list (Printers, under the Native Print menu)
    for the single lifecycle task the sync cannot do: deleting the
    stale row of a renamed or retired printer (audit M2).

    Printing itself NEVER reads this registry: the target printer is
    resolved live on the user's PC through the desktop bridge (the
    live model). This is a log index, not a device inventory.
    """

    _name = "native.print.device"
    _description = "Native Print: printer seen in the job log"
    _order = "name"

    name = fields.Char(
        required=True,
        index=True,
        string="Printer",
    )
    company_id = fields.Many2one(
        "res.company",
        required=True,
        index=True,
        default=lambda s: s.env.company,
        string="Company",
    )
    # Capability snapshot (2026-11-05): the desktop app reports what
    # the printer's own driver declares — protocol, class, paper
    # forms and trays — so the printer arrives in Odoo WITH its sizes
    # and trays attached. Read-only reference for people browsing the
    # log; printing still resolves everything live on the PC and
    # never reads these fields.
    protocol = fields.Char(string="Protocol")
    printer_class = fields.Char(string="Printer Class")
    forms_summary = fields.Text(string="Paper Sizes")
    trays_summary = fields.Text(string="Trays")
    caps_synced_at = fields.Datetime(string="Capabilities Synced")

    _name_company_uniq = models.Constraint(
        "unique(name, company_id)",
        "This printer is already registered for this company.",
    )

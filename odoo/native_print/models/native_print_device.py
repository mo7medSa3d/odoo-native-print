import logging

from odoo import api, fields, models

_logger = logging.getLogger(__name__)


class NativePrintDevice(models.Model):
    """Registry of printers that have actually printed, per company.

    This model exists for ONE reason: so a print job's printer is a
    real Many2one the user can pick, filter and group by in the job
    log, instead of a free-text string nobody should have to type or
    memorise. Rows are created automatically by the job-status sync
    the first time a printer produces a job; there is no
    configuration and no manual workflow for it. Managers get one
    small maintenance list (Printers, under the Native Print menu)
    to delete stale rows for renamed or retired printers.

    Printing itself NEVER reads this registry: the target printer is
    resolved live on the user's PC through the desktop bridge (the
    live model). This is a log index, not a device inventory.
    """

    _name = "native.print.device"
    _description = "Native Print: printer seen in the job log"
    _order = "name"

    @api.model
    def _register_hook(self):
        super()._register_hook()
        try:
            with self.env.cr.savepoint():
                self.env.cr.execute(
                    "SELECT 1 FROM information_schema.tables "
                    "WHERE table_schema = current_schema() AND table_name = %s",
                    [self._table],
                )
                if not self.env.cr.fetchone():
                    return
                self.env.cr.execute(
                    "SELECT column_name FROM information_schema.columns "
                    "WHERE table_schema = current_schema() AND table_name = %s",
                    [self._table],
                )
                existing = {row[0] for row in self.env.cr.fetchall()}
                expected = {
                    "protocol": "varchar",
                    "printer_class": "varchar",
                    "forms_summary": "text",
                    "trays_summary": "text",
                    "caps_synced_at": "timestamp without time zone",
                }
                for col, col_type in expected.items():
                    if col not in existing:
                        self.env.cr.execute(
                            f'ALTER TABLE "{self._table}" ADD COLUMN IF NOT EXISTS "{col}" {col_type}'
                        )
                        _logger.warning(
                            "native_print: added missing column %s.%s automatically",
                            self._table,
                            col,
                        )
        except Exception as exc:
            _logger.debug("native_print: device schema check skipped (%s)", exc)

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
    # Read-only driver capability snapshot synced from the desktop app;
    # printing always resolves capabilities live on the workstation.
    protocol = fields.Char(string="Protocol")
    printer_class = fields.Char(string="Printer Class")
    forms_summary = fields.Text(string="Paper Sizes")
    trays_summary = fields.Text(string="Trays")
    caps_synced_at = fields.Datetime(string="Capabilities Synced")

    _name_company_uniq = models.Constraint(
        "unique(name, company_id)",
        "This printer is already registered for this company.",
    )

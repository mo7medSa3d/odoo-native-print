from datetime import timedelta
import json
import logging

from odoo import _, api, fields, models, tools
from odoo.exceptions import AccessError, UserError

_logger = logging.getLogger(__name__)


class NativePrintJob(models.Model):
    _name = "native.print.job"
    _description = "Yaseir Print: job log"
    _order = "create_date desc, id desc"

    @api.model
    def _register_hook(self):
        super()._register_hook()
        # Defensive schema guard: if the module files on disk were
        # updated and Odoo restarted without `-u native_print`, ensure
        # newly added columns exist before any ORM read/prefetch runs.
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
                    "device_id": "integer",
                    "source_type": "varchar DEFAULT 'report'",
                    "source_model": "varchar",
                    "source_ids": "varchar",
                    "copies": "integer DEFAULT 1",
                    "paper": "varchar",
                    "duplex": "boolean DEFAULT false",
                    "duplex_configured": "boolean DEFAULT false",
                    "color": "boolean DEFAULT true",
                    "mode": "varchar",
                    "report_data": "text",
                }
                for col, col_type in expected.items():
                    if col not in existing:
                        self.env.cr.execute(
                            f'ALTER TABLE "{self._table}" ADD COLUMN IF NOT EXISTS "{col}" {col_type}'
                        )
                        existing.add(col)
                        _logger.warning(
                            "native_print: added missing column %s.%s automatically (run -u native_print to complete module upgrade)",
                            self._table,
                            col,
                        )
                # Composite indexes for /native_print/overview (company_id, state)
                # and _cron_prune_job_log (state, create_date) at 50k+ rows.
                # Check pg_indexes first so worker restarts take no SHARE lock once present.
                self.env.cr.execute(
                    "SELECT indexname FROM pg_indexes "
                    "WHERE schemaname = current_schema() AND tablename = %s",
                    [self._table],
                )
                existing_indexes = {row[0] for row in self.env.cr.fetchall()}
                expected_indexes = (
                    (f"{self._table}_company_state_idx", ("company_id", "state")),
                    (f"{self._table}_state_create_date_idx", ("state", "create_date")),
                )
                for idx_name, cols in expected_indexes:
                    if idx_name not in existing_indexes and set(cols) <= existing:
                        col_sql = ", ".join(f'"{c}"' for c in cols)
                        self.env.cr.execute(
                            f'CREATE INDEX IF NOT EXISTS "{idx_name}" '
                            f'ON "{self._table}" ({col_sql})'
                        )
                        _logger.info(
                            "native_print: created composite index %s on %s(%s)",
                            idx_name,
                            self._table,
                            ", ".join(cols),
                        )
        except Exception as exc:
            _logger.debug("native_print: schema check skipped (%s)", exc)

    name = fields.Char(
        required=True,
        index=True,
        copy=False,
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
    # False in duplex is a real, explicit simplex choice only when this
    # marker is true. Older records and print paths that never set duplex
    # must preserve the printer driver's default when reprinted.
    duplex_configured = fields.Boolean(default=False, copy=False)
    color = fields.Boolean(default=True, string="Color")
    mode = fields.Char(string="Print Mode")
    user_id = fields.Many2one(
        "res.users",
        index=True,
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
    report_data = fields.Text(
        string="Report Data",
        copy=False,
    )
    pages = fields.Integer(default=1, string="Pages")
    byte_size = fields.Integer(string="Bytes")
    attempts = fields.Integer(default=1, string="Attempts")
    company_id = fields.Many2one(
        "res.company",
        required=True,
        index=True,
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
        # Enforce both model ACL and record rules (own-jobs rule for plain
        # users; managers have full company read access).
        self.check_access("read")
        if (
            not self.env.su
            and self.company_id
            and self.company_id not in self.env.user.company_ids
        ):
            raise AccessError(_(
                "You do not have access to this company's print jobs."
            ))
        ids = []
        if self.source_ids:
            seen = set()
            for raw in self.source_ids.split(","):
                part = raw.strip()
                if part.isascii() and part.isdigit():
                    rec_id = int(part)
                    if 1 <= rec_id <= 2_147_483_647 and rec_id not in seen:
                        seen.add(rec_id)
                        ids.append(rec_id)
                        if len(ids) >= 200:
                            break
        # Report metadata is read with sudo: print users have no ACL on
        # ir.actions.report. Authorization is decided explicitly by the
        # group check below, not by that ACL.
        report = self.report_id.sudo()
        if not report and self.report_name:
            found = self.env["ir.actions.report"].sudo().search(
                [("report_name", "=", self.report_name)], limit=1)
            report = found
        if not report or not ids:
            raise UserError(_(
                "This job cannot be reprinted because its report or "
                "records were not saved. Reprint POS receipts directly from POS."
            ))
        # Never trust client-reported source_model: derive model from the
        # report itself, enforce report group restrictions (expanding implied
        # groups since Odoo 19 user.groups_id holds direct groups only),
        # and verify record read access.
        if report.groups_id and not self.env.user._is_admin():
            user_groups = self.env.user.sudo().groups_id
            frontier = user_groups
            while frontier:
                implied = frontier.mapped("implied_ids") - user_groups
                if not implied:
                    break
                user_groups |= implied
                frontier = implied
            if not (report.groups_id & user_groups):
                raise UserError(_(
                    "You do not have access to the report used by this job."
                ))
        model = report.model
        is_product_label_report = (report.report_name or "").startswith(
            ("product.report_product_label_", "product.report_producttemplatelabel")
        )
        if (
            is_product_label_report
            and self.source_model in ("product.template", "product.product", "product.uom")
            and self.source_model in self.env
            and not self.env[self.source_model]._abstract
        ):
            model = self.source_model
        if not model or model not in self.env or self.env[model]._abstract:
            raise UserError(_(
                "This job cannot be reprinted: its report has no valid model."
            ))
        records = self.env[model].browse(ids).exists()
        if not records:
            raise UserError(_(
                "This job cannot be reprinted: the original records "
                "no longer exist or are not accessible."
            ))
        try:
            records.check_access("read")
        except AccessError:
            raise UserError(_(
                "This job cannot be reprinted: the original records "
                "no longer exist or are not accessible."
            )) from None
        parsed_data = None
        if self.report_data:
            try:
                candidate = json.loads(self.report_data)
                if isinstance(candidate, dict) and candidate:
                    parsed_data = candidate
            except (ValueError, TypeError):
                parsed_data = None
        if is_product_label_report:
            wizard_id = parsed_data.get("layout_wizard") if parsed_data else None
            try:
                wizard_rec_id = int(wizard_id) if wizard_id else 0
            except (TypeError, ValueError):
                wizard_rec_id = 0
            wizard_model = self.env.get("product.label.layout")
            wizard_missing = bool(wizard_id) and (
                wizard_rec_id <= 0
                or wizard_model is None
                or not wizard_model.sudo().browse(wizard_rec_id).exists()
            )
            if not parsed_data or wizard_missing:
                raise UserError(_(
                    "This label job cannot be reprinted because its label layout data "
                    "is no longer available. Open the product and click Print -> Labels instead."
                ))
        ids = records.ids
        settings = {}
        if self.copies and self.copies > 0:
            settings["copies"] = self.copies
        if self.paper:
            settings["paper"] = self.paper
        if self.duplex_configured:
            settings["duplex"] = bool(self.duplex)
        settings["color"] = bool(self.color)
        printer_name = (self.device_id.name if self.device_id
                        else self.device)
        action = {
            "type": "ir.actions.report",
            "report_name": report.report_name,
            "report_type": report.report_type,
            "ids": ids,
            "context": {
                "active_ids": ids,
                "active_id": ids[0],
                "active_model": model,
                # Prefill for the print handler: reprint on the job's
                # own printer (when it is connected to this PC) with
                # the job's own settings, not today's selections.
                "native_print_prefill": {
                    "printer": printer_name or None,
                    "settings": settings,
                },
            },
        }
        if parsed_data:
            action["data"] = parsed_data
        return action

    _sql_constraints = [
        (
            "idempotency_key_company_uniq",
            "unique(name, company_id)",
            "A print job with this idempotency key already exists for this company.",
        ),
    ]

    @api.model
    def _cron_prune_job_log(
        self, days=180, batch_size=1000, max_batches=10, stale_active_days=7
    ):
        """Retention hook for the scheduled action (shipped
        INACTIVE — see data/native_print_cron.xml): delete
        terminal-state jobs older than `days` and orphaned active jobs
        untouched for more than `stale_active_days`. The log is an audit
        trail, so pruning is an explicit admin decision rather than a
        silent default."""
        days = max(1, int(days))
        batch_size = max(1, int(batch_size))
        max_batches = max(1, int(max_batches))
        stale_active_days = max(1, int(stale_active_days))
        now = fields.Datetime.now()
        cutoff = fields.Datetime.to_string(now - timedelta(days=days))
        stale_cutoff = fields.Datetime.to_string(
            now - timedelta(days=stale_active_days)
        )
        domain = [
            "|",
            "&",
            ("state", "in", ["done", "failed", "canceled", "dead_letter"]),
            ("create_date", "<", cutoff),
            "&",
            ("state", "in", ["queued", "rendering", "spooling", "printing"]),
            ("write_date", "<", stale_cutoff),
        ]
        Job = self.sudo()
        count = 0
        for _batch in range(max_batches):
            batch = Job.search(domain, order="create_date asc, id asc", limit=batch_size)
            if not batch:
                break
            count += len(batch)
            batch.unlink()
            if not tools.config["test_enable"]:
                self.env.cr.commit()
            if len(batch) < batch_size:
                break
        return count

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
                        "Print job status is synced from the desktop app "
                        "and cannot be edited manually. Reprint the document "
                        "from Odoo instead."
                    ))
        return super().write(vals)

from odoo.exceptions import UserError
from odoo.tests.common import TransactionCase


class TestNativePrintJobLog(TransactionCase):
    """The server keeps ONLY the job log: printers are live on the
    user's PC and are never modelled or stored here."""

    def setUp(self):
        super().setUp()
        self.Job = self.env["native.print.job"]

    def _job(self, key="job-key-1"):
        return self.Job.create({
            "name": key,
            "state": "queued",
            "device": "EPSON TM-T88VII",
            "report_name": "account.report_invoice",
        })

    def test_duplicate_job_key_is_rejected(self):
        self._job()
        with self.assertRaises(Exception):
            self._job()

    def test_job_state_changes_only_via_status_sync(self):
        job = self._job()
        with self.assertRaises(UserError):
            job.write({"state": "done"})
        job.with_context(native_print_status_sync=True).write(
            {"state": "done"})
        self.assertEqual(job.state, "done")

    def test_action_reprint_returns_report_action(self):
        report = self.env["ir.actions.report"].search(
            [("report_name", "=", "account.report_invoice")], limit=1)
        if not report:
            self.skipTest("account reports not installed")
        job = self._job()
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_model": "account.move",
            "source_ids": "41,42",
        })
        action = job.action_reprint()
        self.assertEqual(action["type"], "ir.actions.report")
        self.assertEqual(action["report_name"], "account.report_invoice")
        self.assertEqual(action["ids"], [41, 42])
        self.assertEqual(action["context"]["active_model"], "account.move")

    def test_action_reprint_carries_prefill(self):
        report = self.env["ir.actions.report"].search(
            [("report_name", "=", "account.report_invoice")], limit=1)
        if not report:
            self.skipTest("account reports not installed")
        job = self._job()
        printer = self.env["native.print.device"].create({
            "name": "EPSON TM-T88VII",
            "company_id": job.company_id.id,
        })
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_model": "account.move",
            "source_ids": "41",
            "device_id": printer.id,
            "copies": 2,
            "paper": "A5",
        })
        prefill = job.action_reprint()["context"]["native_print_prefill"]
        self.assertEqual(prefill["printer"], "EPSON TM-T88VII")
        self.assertEqual(prefill["settings"]["copies"], 2)
        self.assertEqual(prefill["settings"]["paper"], "A5")

    def test_action_reprint_without_source_fails_loudly(self):
        from odoo.exceptions import UserError
        job = self._job()
        with self.assertRaises(UserError):
            job.action_reprint()

    def test_job_defaults_to_session_company_and_user(self):
        job = self._job("job-key-2")
        self.assertEqual(job.company_id, self.env.company)
        self.assertEqual(job.user_id, self.env.user)

    def test_device_registry_is_read_only_for_everyone(self):
        # The registry is auto-synced history (audit F6): neither the
        # user nor the manager group may create/write/unlink rows by
        # hand — a manual rename or delete would corrupt the job log's
        # printer history. Creation happens only via the controller's
        # sudo() registration path.
        access = self.env["ir.model.access"].search([
            ("model_id.model", "=", "native.print.device"),
        ])
        self.assertTrue(access, "device access rules must exist")
        for rule in access:
            self.assertTrue(rule.perm_read)
            self.assertFalse(rule.perm_create, rule.name)
            self.assertFalse(rule.perm_write, rule.name)
            self.assertFalse(rule.perm_unlink, rule.name)

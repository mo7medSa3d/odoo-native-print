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

    def test_job_defaults_to_session_company_and_user(self):
        job = self._job("job-key-2")
        self.assertEqual(job.company_id, self.env.company)
        self.assertEqual(job.user_id, self.env.user)

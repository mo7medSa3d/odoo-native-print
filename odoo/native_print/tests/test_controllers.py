from odoo.tests import HttpCase, tagged

JOB_KEY = "01234567-89ab-4cde-8abc-0123456789ab"


@tagged("post_install", "-at_install")
class TestNativePrintControllers(HttpCase):
    """Controller tests for the live model: the server exposes only
    the job log (job_status + overview). Printers are discovered live
    through the desktop bridge and never stored server-side."""

    def _rpc(self, path, params):
        res = self.url_open(path, data=self._json(params),
                            headers={"Content-Type": "application/json"}).json()
        self.assertNotIn("error", res, res)
        return res["result"]

    def _json(self, params):
        import json as _json

        return _json.dumps({"jsonrpc": "2.0", "method": "call",
                            "params": params})

    def setUp(self):
        super().setUp()
        self.authenticate("admin", "admin")

    def test_no_printer_storage_endpoints_exist(self):
        # The removed routes must stay removed: printers are live.
        for path in ("/native_print/sync_printers",
                     "/native_print/rules",
                     "/native_print/set_default_printer"):
            res = self.url_open(path, data=self._json({}),
                                headers={"Content-Type": "application/json"})
            self.assertEqual(res.status_code, 404, path)

    def test_job_status_lifecycle_and_terminal_finality(self):
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "queued",
            "device": "EPSON TM-T88VII",
            "report_name": "account.report_invoice",
        })
        self.assertTrue(res["ok"], res)
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "done",
        })
        self.assertEqual(res["state"], "done")
        # A late failure post must never rewrite a finished job.
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "failed",
            "error_code": "LATE",
        })
        self.assertEqual(res["state"], "done")
        self.assertEqual(res.get("ignored"), "terminal_state_is_final")
        job = self.env["native.print.job"].search(
            [("name", "=", JOB_KEY)])
        self.assertEqual(job.state, "done")
        self.assertEqual(job.device, "EPSON TM-T88VII")
        self.assertEqual(job.report_name, "account.report_invoice")

    def test_job_status_validation(self):
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": "k" * 200, "state": "queued",
        })
        self.assertFalse(res["ok"])
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": "short-key", "state": "bogus",
        })
        self.assertFalse(res["ok"])

    def test_overview_counts_jobs(self):
        self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "queued",
        })
        overview = self._rpc("/native_print/overview", {})
        self.assertTrue(overview["ok"])
        self.assertEqual(overview["active_jobs"], 1)
        self.assertEqual(overview["done_jobs"], 0)
        self.assertEqual(overview["failed_jobs"], 0)

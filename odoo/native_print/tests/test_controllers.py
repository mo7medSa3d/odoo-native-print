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

    def test_job_status_state_machine(self):
        key = "11111111-2222-4333-8444-555555555555"
        self._rpc("/native_print/job_status", {
            "idempotency_key": key, "state": "queued", "attempts": 1,
        })
        self._rpc("/native_print/job_status", {
            "idempotency_key": key, "state": "printing",
        })
        # Backward move: ignored, recorded state does not regress.
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": key, "state": "rendering",
        })
        self.assertEqual(res.get("ignored"), "illegal_transition")
        self.assertEqual(res["state"], "printing")
        # A job that is already printing cannot be canceled.
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": key, "state": "canceled",
        })
        self.assertEqual(res.get("ignored"), "illegal_transition")
        # Same-state repost is a legal metadata refresh, and attempts
        # never regress.
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": key, "state": "printing", "attempts": 0,
        })
        self.assertTrue(res["ok"])
        job = self.env["native.print.job"].search([("name", "=", key)])
        self.assertEqual(job.state, "printing")
        self.assertEqual(job.attempts, 1)
        # Forward to a terminal state still works from printing.
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": key, "state": "done",
        })
        self.assertEqual(res["state"], "done")

    def test_job_status_validation(self):
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": "k" * 200, "state": "queued",
        })
        self.assertFalse(res["ok"])
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": "short-key", "state": "bogus",
        })
        self.assertFalse(res["ok"])

    def test_job_status_registers_printer_and_source(self):
        # The creating post carries the printer and the source
        # reference: the job's printer becomes a pickable registry
        # entry (never free text to type), and the source is stored
        # for the log's Reprint action.
        self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "queued",
            "device": "EPSON TM-T88VII",
            "report_name": "account.report_invoice",
            "source_type": "report",
            "source_model": "account.move",
            "source_ids": "41,42",
        })
        job = self.env["native.print.job"].search(
            [("name", "=", JOB_KEY)])
        self.assertTrue(job.device_id)
        self.assertEqual(job.device_id.name, "EPSON TM-T88VII")
        self.assertEqual(job.device_id.company_id, job.company_id)
        self.assertEqual(job.source_type, "report")
        self.assertEqual(job.source_model, "account.move")
        self.assertEqual(job.source_ids, "41,42")
        # A second job from the same printer reuses the registry row.
        self._rpc("/native_print/job_status", {
            "idempotency_key": "99999999-8888-4777-8666-555555555555",
            "state": "queued", "device": "EPSON TM-T88VII",
        })
        self.assertEqual(self.env["native.print.device"].search_count(
            [("name", "=", "EPSON TM-T88VII")]), 1)

    def test_job_status_rejects_dirty_source_data(self):
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "queued",
            "source_model": "account.move; DROP TABLE",
            "source_ids": "1,abc,2",
            "source_type": "bogus",
        })
        self.assertTrue(res["ok"], res)
        job = self.env["native.print.job"].search(
            [("name", "=", JOB_KEY)])
        self.assertFalse(job.source_model)
        self.assertEqual(job.source_ids, "1,2")
        self.assertEqual(job.source_type, "report")

    def test_job_status_stores_print_settings(self):
        self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "queued",
            "device": "EPSON TM-T88VII",
            "report_name": "account.report_invoice",
            "copies": 3, "paper": "A5", "duplex": True,
            "color": False, "mode": "gdi",
        })
        job = self.env["native.print.job"].search(
            [("name", "=", JOB_KEY)])
        self.assertEqual(job.copies, 3)
        self.assertEqual(job.paper, "A5")
        self.assertTrue(job.duplex)
        self.assertFalse(job.color)
        self.assertEqual(job.mode, "gdi")

    def test_late_posts_update_never_duplicate(self):
        # Terminal outcome first (the creating post was lost), then
        # the late "queued" post arrives: it must neither create a
        # second record nor regress the finished job.
        self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "done",
        })
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "queued",
        })
        self.assertEqual(res["state"], "done")
        self.assertEqual(self.env["native.print.job"].search_count(
            [("name", "=", JOB_KEY)]), 1)
        # Repeated identical posts keep updating the same record.
        self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "done",
        })
        self.assertEqual(self.env["native.print.job"].search_count(
            [("name", "=", JOB_KEY)]), 1)

    def test_overview_counts_jobs(self):
        self._rpc("/native_print/job_status", {
            "idempotency_key": JOB_KEY, "state": "queued",
        })
        overview = self._rpc("/native_print/overview", {})
        self.assertTrue(overview["ok"])
        self.assertEqual(overview["active_jobs"], 1)
        self.assertEqual(overview["done_jobs"], 0)
        self.assertEqual(overview["failed_jobs"], 0)


@tagged("post_install", "-at_install")
class TestNativePrintMultiUser(HttpCase):
    """The same guarantees for NON-admin users (audit L14), including
    the audit H1 scenario: two cashiers in one company printing the
    same POS order must never collide, and users of another company
    stay fully isolated. The client now sends per-user keys; these
    tests pin the server-side guards that back that up."""

    def _rpc(self, path, params):
        res = self.url_open(path, data=self._json(params),
                            headers={"Content-Type": "application/json"}).json()
        self.assertNotIn("error", res, res)
        return res["result"]

    def _json(self, params):
        import json as _json

        return _json.dumps({"jsonrpc": "2.0", "method": "call",
                            "params": params})

    def _make_user(self, login, company):
        group = self.env.ref("native_print.group_native_print_user")
        user = self.env["res.users"].sudo().create({
            "name": login,
            "login": login,
            "password": "np-test-password",
            "company_id": company.id,
            "company_ids": [company.id],
            "group_ids": [(4, group.id), (4, self.env.ref("base.group_user").id)],
        })
        return user

    def setUp(self):
        super().setUp()
        self.company = self.env.company
        self.other_company = self.env["res.company"].sudo().create({
            "name": "NP Other Company",
        })
        self.cashier_a = self._make_user("np_cashier_a", self.company)
        self.cashier_b = self._make_user("np_cashier_b", self.company)
        self.outsider = self._make_user("np_outsider", self.other_company)

    def test_second_cashier_cannot_touch_first_cashiers_job(self):
        key = "pos-Order_1-20261104-u1-a1"
        self.authenticate("np_cashier_a", "np-test-password")
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": key, "state": "queued",
            "source_type": "pos_receipt",
        })
        self.assertTrue(res["ok"], res)
        self.authenticate("np_cashier_b", "np-test-password")
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": key, "state": "done",
        })
        self.assertEqual(res, {"ok": False, "error": "forbidden"})
        job = self.env["native.print.job"].sudo().search(
            [("name", "=", key)])
        self.assertEqual(job.state, "queued")
        self.assertEqual(job.user_id, self.cashier_a)
        # Her own key for the same order is a separate, working job.
        own_key = "pos-Order_1-20261104-u2-a1"
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": own_key, "state": "done",
            "source_type": "pos_receipt",
        })
        self.assertTrue(res["ok"], res)
        self.assertEqual(res["state"], "done")

    def test_overview_is_company_wide_for_cashiers(self):
        self.authenticate("np_cashier_a", "np-test-password")
        self._rpc("/native_print/job_status", {
            "idempotency_key": "k-cashier-a", "state": "done",
        })
        self.authenticate("np_cashier_b", "np-test-password")
        overview = self._rpc("/native_print/overview", {})
        self.assertTrue(overview["ok"])
        # Company-wide counters (audit L1): B sees the company's
        # totals, matching the systray labels, not just his own jobs.
        self.assertEqual(overview["done_jobs"], 1)

    def test_other_company_is_isolated(self):
        key = "shared-key-name"
        self.authenticate("np_cashier_a", "np-test-password")
        self._rpc("/native_print/job_status", {
            "idempotency_key": key, "state": "done",
        })
        self.authenticate("np_outsider", "np-test-password")
        # The same key in another company is a different job.
        res = self._rpc("/native_print/job_status", {
            "idempotency_key": key, "state": "queued",
        })
        self.assertTrue(res["ok"], res)
        self.assertEqual(res["state"], "queued")
        jobs = self.env["native.print.job"].sudo().search(
            [("name", "=", key)])
        self.assertEqual(len(jobs), 2)
        overview = self._rpc("/native_print/overview", {})
        self.assertEqual(overview["active_jobs"], 1)
        self.assertEqual(overview["done_jobs"], 0)

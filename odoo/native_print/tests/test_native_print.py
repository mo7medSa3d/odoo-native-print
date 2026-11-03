from odoo.tests import TransactionCase, tagged


@tagged("post_install", "-at_install")
class TestNativePrint(TransactionCase):
    """Addon tests. They run on a real Odoo 19 (spike S2 environment);
    they cannot run on a machine without Odoo, by design."""

    def setUp(self):
        super().setUp()
        self.profile = self.env["native.print.profile"].create({
            "name": "Office A4",
            "printer_pattern": "HP LaserJet",
            "mode": "gdi",
            "paper": "A4",
        })

    def test_profile_defaults_and_counts(self):
        self.assertEqual(self.profile.copies, 1)
        self.assertEqual(self.profile.scale_policy, "custom_allowed")
        self.assertEqual(self.profile.recovery_mode, "safe_no_reprint")
        self.assertEqual(self.profile.company_id, self.env.company)
        self.assertTrue(self.profile.active)
        self.assertEqual(self.profile.rule_count, 0)
        self.assertEqual(self.profile.job_count, 0)
        self.assertEqual(self.profile.failed_job_count, 0)

    def test_job_log_states_and_actions(self):
        job = self.env["native.print.job"].create({
            "name": "test-key-1",
            "state": "queued",
            "pages": 3,
            "profile_id": self.profile.id,
        })
        self.assertEqual(job.state, "queued")
        self.profile.invalidate_recordset()
        self.assertEqual(self.profile.job_count, 1)
        job.action_cancel_job()
        self.assertEqual(job.state, "canceled")
        self.assertEqual(job.error_code, "CANCELED_BY_USER")
        job.action_retry_job()
        self.assertEqual(job.state, "queued")
        self.assertFalse(job.error_code)
        job.action_mark_done()
        self.assertEqual(job.state, "done")

    def test_pos_receipt_rule_by_source_name(self):
        # POS receipts are not ir.actions.report reports; their rule is
        # matched by the free-text source name (controller fallback path).
        rule = self.env["native.print.rule"].create({
            "name": "POS receipts",
            "report_name": "point_of_sale.receipt",
            "profile_id": self.profile.id,
            "priority": 10,
        })
        found = self.env["native.print.rule"].search([
            ("report_name", "=", "point_of_sale.receipt"),
            ("company_id", "=", self.env.company.id),
        ])
        self.assertEqual(found, rule)
        self.assertEqual(rule.target_source_display, "point_of_sale.receipt")

    def test_wildcard_catch_all_rule_display(self):
        catch_all = self.env["native.print.rule"].create({
            "name": "Default Catch-All",
            "profile_id": self.profile.id,
            "priority": 1,
        })
        self.assertEqual(catch_all.target_source_display, "*")

    def test_rule_priority_order(self):
        report = self.env["ir.actions.report"].search([], limit=1)
        if not report:
            self.skipTest("no reports in this database")
        low = self.env["native.print.rule"].create({
            "name": "low",
            "report_id": report.id,
            "profile_id": self.profile.id,
            "priority": 1,
        })
        high = self.env["native.print.rule"].create({
            "name": "high",
            "report_id": report.id,
            "profile_id": self.profile.id,
            "priority": 50,
        })
        rules = self.env["native.print.rule"].search(
            [("report_id", "=", report.id)])
        self.assertEqual(rules[0], high)
        self.assertEqual(rules[-1], low)

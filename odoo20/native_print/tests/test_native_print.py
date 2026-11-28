from odoo.exceptions import AccessError, UserError
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

    def _report_on_partners(self):
        # A synthetic report on a model that always exists, with
        # real records: reprint must work without the account module
        # and must never depend on demo data with fixed ids.
        report = self.env["ir.actions.report"].create({
            "name": "NP Test Partner Report",
            "model": "res.partner",
            "report_type": "qweb-pdf",
            "report_name": "native_print.test_partner_report",
        })
        partners = self.env["res.partner"].create([
            {"name": "NP Reprint A"}, {"name": "NP Reprint B"},
        ])
        return report, partners

    def test_action_reprint_returns_report_action(self):
        report, partners = self._report_on_partners()
        job = self._job()
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            # The posted source_model is client data and may lie:
            # the reprint must still target the REPORT's model
            # (client-reported source_model must never be trusted).
            "source_model": "res.users",
            "source_ids": ",".join(str(i) for i in partners.ids),
        })
        action = job.action_reprint()
        self.assertEqual(action["type"], "ir.actions.report")
        self.assertEqual(action["report_name"], "native_print.test_partner_report")
        self.assertEqual(action["ids"], partners.ids)
        self.assertEqual(action["context"]["active_model"], "res.partner")

    def test_action_reprint_carries_prefill(self):
        report, partners = self._report_on_partners()
        job = self._job()
        printer = self.env["native.print.device"].create({
            "name": "EPSON TM-T88VII",
            "company_id": job.company_id.id,
        })
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_model": "res.partner",
            "source_ids": str(partners[0].id),
            "device_id": printer.id,
            "copies": 2,
            "paper": "A5",
        })
        prefill = job.action_reprint()["context"]["native_print_prefill"]
        self.assertEqual(prefill["printer"], "EPSON TM-T88VII")
        self.assertEqual(prefill["settings"]["copies"], 2)
        self.assertEqual(prefill["settings"]["paper"], "A5")
        self.assertNotIn("duplex", prefill["settings"])

    def test_action_reprint_preserves_explicit_simplex_choice(self):
        report, partners = self._report_on_partners()
        job = self._job()
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_ids": str(partners[0].id),
            "duplex": False,
            "duplex_configured": True,
        })
        settings = job.action_reprint()["context"]["native_print_prefill"]["settings"]
        self.assertIs(settings["duplex"], False)

    def test_action_reprint_preserves_explicit_duplex_choice(self):
        report, partners = self._report_on_partners()
        job = self._job()
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_ids": str(partners[0].id),
            "duplex": True,
            "duplex_configured": True,
        })
        settings = job.action_reprint()["context"]["native_print_prefill"]["settings"]
        self.assertIs(settings["duplex"], True)

    def test_action_reprint_rejects_missing_records(self):
        report, partners = self._report_on_partners()
        job = self._job()
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_ids": str(partners.ids[0]),
        })
        partners.unlink()
        with self.assertRaises(UserError):
            job.action_reprint()

    def test_action_reprint_label_report_requires_and_preserves_report_data(self):
        partners = self.env["res.partner"].create([{"name": "NP Label Item"}])
        report = self.env["ir.actions.report"].create({
            "name": "NP Label Test",
            "model": "res.partner",
            "report_type": "qweb-text",
            "report_name": "product.report_product_label_zpl",
        })
        job = self._job(key="job-label-1")
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "report_name": "product.report_product_label_zpl",
            "source_ids": str(partners[0].id),
        })
        with self.assertRaises(UserError):
            job.action_reprint()
        job.with_context(native_print_status_sync=True).write({
            "report_data": '{"labels":[{"name":"Item"}],"label_template":"product.barcode_normal_label"}',
        })
        action = job.action_reprint()
        self.assertEqual(action["data"]["labels"][0]["name"], "Item")

    def _plain_print_user(self, extra_group_ids=()):
        """A non-admin user who may read the job log but holds no
        special report group (plus any groups the caller adds)."""
        groups = [self.env.ref("base.group_user").id,
                  self.env.ref("native_print.group_native_print_user").id]
        groups.extend(extra_group_ids)
        return self.env["res.users"].create({
            "name": "NP Plain Cashier",
            "login": "np_plain_cashier",
            "group_ids": [(6, 0, groups)],
        })

    def test_action_reprint_respects_report_groups(self):
        report, partners = self._report_on_partners()
        group = self.env["res.groups"].create({"name": "NP Restricted Reports"})
        report.group_ids = [(4, group.id)]
        user = self._plain_print_user()
        job = self._job()
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_ids": str(partners.ids[0]),
            "user_id": user.id,
        })
        # A plain user without the report's group is still refused
        # (restricted reports stay restricted for non-admin users).
        with self.assertRaises(UserError):
            job.with_user(user).action_reprint()

    def test_action_reprint_allows_administrator(self):
        # Administrators must be able to reprint restricted reports
        # without failing on direct-group-only checks.
        report, partners = self._report_on_partners()
        group = self.env["res.groups"].create({"name": "NP Admin Only Reports"})
        report.group_ids = [(4, group.id)]
        job = self._job()
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_ids": str(partners.ids[0]),
        })
        admin = self.env.ref("base.user_admin")
        self.assertTrue(admin._is_admin())
        action = job.with_user(admin).action_reprint()
        self.assertEqual(action.get("type"), "ir.actions.report")

    def test_action_reprint_allows_implied_group_membership(self):
        # Membership through an implying group is real membership:
        # a user whose group implies the report's group qualifies.
        report, partners = self._report_on_partners()
        report_group = self.env["res.groups"].create({"name": "NP Implied Target"})
        parent_group = self.env["res.groups"].create({
            "name": "NP Implying Parent",
            "implied_ids": [(4, report_group.id)],
        })
        report.group_ids = [(4, report_group.id)]
        user = self._plain_print_user(extra_group_ids=[parent_group.id])
        job = self._job()
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_ids": str(partners.ids[0]),
            "user_id": user.id,
        })
        action = job.with_user(user).action_reprint()
        self.assertEqual(action.get("type"), "ir.actions.report")

    def test_action_reprint_rejects_other_users_job(self):
        report, partners = self._report_on_partners()
        job = self._job("job-key-other-user")
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_ids": str(partners.ids[0]),
        })
        with self.assertRaises(AccessError):
            job.with_user(self._plain_print_user()).action_reprint()

    def test_action_reprint_rejects_user_without_model_acl(self):
        portal_user = self.env["res.users"].create({
            "name": "NP Portal User",
            "login": "np_portal_user",
            "group_ids": [(6, 0, [self.env.ref("base.group_portal").id])],
        })
        job = self._job("job-key-portal")
        with self.assertRaises(AccessError):
            job.with_user(portal_user).action_reprint()

    def test_action_reprint_rejects_other_company_user(self):
        other_company = self.env["res.company"].create({"name": "NP Isolated Co"})
        outsider = self.env["res.users"].create({
            "name": "NP Other Company User",
            "login": "np_other_company_user",
            "company_id": other_company.id,
            "company_ids": [(6, 0, [other_company.id])],
            "group_ids": [(6, 0, [
                self.env.ref("base.group_user").id,
                self.env.ref("native_print.group_native_print_user").id,
            ])],
        })
        report, partners = self._report_on_partners()
        job = self._job("job-key-cross-company")
        job.with_context(native_print_status_sync=True).write({
            "report_id": report.id,
            "source_ids": str(partners.ids[0]),
        })
        with self.assertRaises(AccessError):
            job.with_user(outsider).action_reprint()

    def test_action_reprint_without_source_fails_loudly(self):
        job = self._job()
        with self.assertRaises(UserError):
            job.action_reprint()

    def test_job_defaults_to_session_company_and_user(self):
        job = self._job("job-key-2")
        self.assertEqual(job.company_id, self.env.company)
        self.assertEqual(job.user_id, self.env.user)

    def test_device_registry_access_policy(self):
        # Odoo 20 unified ir.access + runtime check_access verification:
        # nobody may create or write rows by hand; only managers may unlink stale rows.
        access = self.env["ir.access"].search([
            ("model_id.model", "=", "native.print.device"),
        ])
        self.assertTrue(access, "device ir.access rules must exist")

        Device = self.env["native.print.device"]
        plain_user = self._plain_print_user()
        manager = self.env["res.users"].create({
            "name": "NP Manager",
            "login": "np_device_manager",
            "group_ids": [(6, 0, [
                self.env.ref("base.group_user").id,
                self.env.ref("native_print.group_native_print_manager").id,
            ])],
        })
        device = Device.create({
            "name": "NP Access Test Printer",
            "company_id": self.env.company.id,
        })

        device.with_user(plain_user).check_access("read")
        for op in ("create", "write", "unlink"):
            with self.assertRaises(AccessError, msg=f"plain user must not {op} device"):
                device.with_user(plain_user).check_access(op)

        device.with_user(manager).check_access("read")
        device.with_user(manager).check_access("unlink")
        for op in ("create", "write"):
            with self.assertRaises(AccessError, msg=f"manager must not {op} device"):
                device.with_user(manager).check_access(op)

    def test_prune_job_log_removes_only_old_terminal_jobs(self):
        old_done = self._job("prune-old-done")
        old_active = self._job("prune-old-active")
        recent_done = self._job("prune-recent-done")
        for job in (old_done, old_active, recent_done):
            job.with_context(native_print_status_sync=True).write(
                {"state": "done" if "done" in job.name else "queued"})
        self.env.cr.execute(
            "UPDATE native_print_job SET create_date = "
            "create_date - interval '200 days' WHERE id IN %s",
            (tuple((old_done + old_active).ids),),
        )
        self.Job._cron_prune_job_log(days=180)
        remaining = self.Job.search(
            [("name", "like", "prune-")]).mapped("name")
        self.assertNotIn("prune-old-done", remaining)
        self.assertIn("prune-old-active", remaining)
        self.assertIn("prune-recent-done", remaining)

    def test_composite_indexes_exist(self):
        self.env.cr.execute(
            "SELECT indexname FROM pg_indexes "
            "WHERE schemaname = current_schema() AND tablename = %s",
            [self.Job._table],
        )
        indexes = {row[0] for row in self.env.cr.fetchall()}
        self.assertIn(f"{self.Job._table}_company_state_idx", indexes)
        self.assertIn(f"{self.Job._table}_state_create_date_idx", indexes)


class TestTranslationFiles(TransactionCase):
    """Regression guard for the Arabic catalog: Odoo's PO reader
    requires a `module:` comment on every entry — a single entry
    without one crashes /web/webclient/translations with a 500,
    which blocks the web client boot (white page) for Arabic users.
    The web catalog must also actually serve the JS terms."""

    def test_arabic_catalog_loads_and_serves_web_terms(self):
        from odoo.tools.translate import code_translations

        web = code_translations.get_web_translations("native_print", "ar_001")
        messages = web.get("messages", ())
        self.assertGreaterEqual(len(messages), 20)
        python_terms = code_translations.get_python_translations(
            "native_print", "ar_001"
        )
        self.assertTrue(python_terms)

    def test_arabic_catalog_hygiene_and_code_coverage(self):
        """Every catalog entry must be importable (module comment +
        at least one occurrence: entries without occurrences are
        silently dropped by Odoo's importer and their strings fall
        back to English), and every translatable literal in the
        module's JS/Python sources must exist in the catalog."""
        import ast
        import os
        import re

        from odoo.modules.module import get_module_path

        root = get_module_path("native_print")
        po_path = os.path.join(root, "i18n", "ar.po")
        with open(po_path, encoding="utf-8") as fh:
            text = fh.read()

        def po_strings(block, keyword):
            parts = []
            lines = block.splitlines()
            for idx, line in enumerate(lines):
                if line.startswith(keyword + " "):
                    parts.append(ast.literal_eval(line[len(keyword) + 1:].strip()))
                    for nxt in lines[idx + 1:]:
                        if nxt.startswith('"'):
                            parts.append(ast.literal_eval(nxt.strip()))
                        else:
                            break
                    break
            return "".join(parts)

        msgids = set()
        for block in text.split("\n\n"):
            if 'msgid "' not in block:
                continue
            msgid = po_strings(block, "msgid")
            if not msgid:
                continue  # header
            self.assertIn(
                "#. module: native_print", block,
                f"catalog entry without module comment: {msgid!r}",
            )
            self.assertTrue(
                any(l.startswith("#:") for l in block.splitlines()),
                f"catalog entry without occurrences (importer drops it): {msgid!r}",
            )
            if any("/static/src/js/" in l for l in block.splitlines() if l.startswith("#:")):
                self.assertIn(
                    "#. odoo-javascript", block,
                    f"JS catalog entry without odoo-javascript marker: {msgid!r}",
                )
            msgids.add(msgid)
        self.assertGreaterEqual(len(msgids), 100)

        # One call may span several adjacent literals (implicit
        # concatenation in Python): capture the whole argument and
        # join its fragments, like Odoo's own extractor does.
        call_re = re.compile(r"_t?\(\s*((?:(['\"])((?:\\.|(?!\2).)*)\2\s*)+)")
        frag_re = re.compile(r"(['\"])((?:\\.|(?!\1).)*)\1")
        for sub, pattern in (
            (os.path.join("static", "src", "js"), "*.js"),
            ("controllers", "*.py"),
            ("models", "*.py"),
        ):
            folder = os.path.join(root, sub)
            if not os.path.isdir(folder):
                continue
            for name in sorted(os.listdir(folder)):
                if not name.endswith(pattern[1:]):
                    continue
                with open(os.path.join(folder, name), encoding="utf-8") as fh:
                    src = fh.read()
                for call in call_re.finditer(src):
                    # ast.literal_eval on each re-quoted fragment
                    # decodes the escapes without mangling UTF-8 text.
                    literal = "".join(
                        ast.literal_eval(q + raw + q)
                        for q, raw in frag_re.findall(call.group(1))
                    )
                    self.assertIn(
                        literal, msgids,
                        f"{name}: translatable literal missing from ar.po: {literal!r}",
                    )

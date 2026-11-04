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

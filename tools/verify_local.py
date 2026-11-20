#!/usr/bin/env python3
"""One-command offline local verifier for odoo-native-print.

Runs version/command-surface gates, shell syntax, Node tests, Python compile,
XML/ADMX/ADML well-formedness, TOML/JSON syntax, and ar.po msgid checks with
per-step timings and a PASS/FAIL summary. Zero third-party dependencies.
"""
import ast
import json
import py_compile
import subprocess
import sys
import time
import tomllib
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = {".git", "node_modules", "target", "__pycache__"}


def _iter_files(suffixes):
    for p in sorted(ROOT.rglob("*")):
        if p.is_file() and p.suffix in suffixes and not (SKIP_DIRS & set(p.parts)):
            yield p


def _run_cmd(cmd):
    proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if proc.returncode != 0:
        out = (proc.stdout + "\n" + proc.stderr).strip()
        tail = "\n".join(out.splitlines()[-15:]) if out else f"exit {proc.returncode}"
        return False, tail
    return True, ""


def _check_py_compile():
    files = sorted(ROOT.glob("odoo/native_print/**/*.py")) + sorted(ROOT.glob("tools/*.py"))
    if not files:
        return False, "no Python files found"
    for f in files:
        py_compile.compile(str(f), doraise=True)
    return True, f"{len(files)} files"


def _check_xml():
    files = list(_iter_files({".xml", ".admx", ".adml"}))
    if not files:
        return False, "no XML/ADMX/ADML files found"
    for f in files:
        ET.parse(f)
    return True, f"{len(files)} files"


def _check_toml_json():
    toml_files = list(_iter_files({".toml"}))
    json_files = list(_iter_files({".json"}))
    for f in toml_files:
        tomllib.loads(f.read_text(encoding="utf-8"))
    for f in json_files:
        json.loads(f.read_text(encoding="utf-8"))
    return True, f"{len(toml_files)} toml, {len(json_files)} json"


def parse_po_entries(text):
    """Parse (lineno, msgctxt, msgid) entries from GNU gettext .po text."""
    entries = []
    ctx, mid, mid_line, field = None, None, None, None

    def flush():
        nonlocal ctx, mid, mid_line, field
        if mid is not None:
            entries.append((mid_line, ctx, mid))
        ctx, mid, mid_line, field = None, None, None, None

    for lineno, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line:
            flush()
            continue
        if line.startswith("#"):
            continue
        if line.startswith("msgctxt "):
            if mid is not None:
                flush()
            ctx = ast.literal_eval(line[8:].strip())
            field = "msgctxt"
        elif line.startswith("msgid "):
            if mid is not None:
                flush()
            mid = ast.literal_eval(line[6:].strip())
            mid_line = lineno
            field = "msgid"
        elif line.startswith(("msgstr", "msgid_plural")):
            field = "other"
        elif line.startswith('"'):
            piece = ast.literal_eval(line)
            if field == "msgid":
                mid += piece
            elif field == "msgctxt":
                ctx += piece
    flush()
    return entries


def check_po_text(text):
    entries = parse_po_entries(text)
    seen = {}
    errors = []
    for idx, (lineno, ctx, mid) in enumerate(entries):
        if mid == "":
            if idx == 0 and ctx is None:
                continue  # standard PO metadata header
            errors.append(f"line {lineno}: empty msgid")
            continue
        key = (ctx, mid)
        if key in seen:
            errors.append(f"line {lineno}: duplicate msgid (first at line {seen[key]}): {mid!r}")
        else:
            seen[key] = lineno
    return len(entries), errors


def _check_ar_po():
    po_path = ROOT / "odoo/native_print/i18n/ar.po"
    count, errors = check_po_text(po_path.read_text(encoding="utf-8"))
    if errors:
        return False, "\n".join(errors[:10])
    return True, f"{count} entries"


def _self_test():
    n, errs = check_po_text('msgid ""\nmsgstr "Header\\n"\n\nmsgid "Hello"\nmsgstr "مرحبا"\n')
    assert n == 2 and not errs, (n, errs)
    _, errs = check_po_text('msgid ""\nmsgstr ""\n\nmsgid ""\nmsgstr "bad"\n')
    assert len(errs) == 1 and "empty msgid" in errs[0], errs
    _, errs = check_po_text('msgid "Dup"\nmsgstr "1"\n\nmsgid "Dup"\nmsgstr "2"\n')
    assert len(errs) == 1 and "duplicate msgid" in errs[0], errs
    print("self-test OK")
    return 0


def main():
    if "--self-test" in sys.argv[1:]:
        return _self_test()

    steps = [
        ("versions", lambda: _run_cmd([sys.executable, "tools/check_versions.py"])),
        ("command_surface", lambda: _run_cmd([sys.executable, "tools/check_command_surface.py"])),
        ("bash_syntax:fetch-pdfium", lambda: _run_cmd(["bash", "-n", "tools/fetch-pdfium.sh"])),
        ("node:control-center", lambda: _run_cmd(["node", "--test", "app/tests/control-center.test.mjs"])),
        ("node:pos_keys", lambda: _run_cmd(["node", "--test", "odoo/native_print/tests/test_pos_keys.mjs"])),
        ("node:live_target", lambda: _run_cmd(["node", "--test", "odoo/native_print/tests/test_live_target.mjs"])),
        ("python:py_compile", _check_py_compile),
        ("xml/admx/adml", _check_xml),
        ("toml/json", _check_toml_json),
        ("i18n:ar.po", _check_ar_po),
    ]

    total_t0 = time.perf_counter()
    passed = 0
    failures = []

    for name, fn in steps:
        t0 = time.perf_counter()
        try:
            ok, detail = fn()
        except Exception as exc:
            ok, detail = False, f"{type(exc).__name__}: {exc}"
        dt_ms = (time.perf_counter() - t0) * 1000.0
        status = "PASS" if ok else "FAIL"
        suffix = f" ({detail})" if ok and detail else ""
        print(f"[{status}] {name:<26} {dt_ms:7.1f} ms{suffix}")
        if ok:
            passed += 1
        else:
            failures.append((name, detail))

    total_s = time.perf_counter() - total_t0
    print(f"\nSummary: {passed}/{len(steps)} passed in {total_s:.2f}s")
    for name, detail in failures:
        print(f"\n--- FAIL: {name} ---\n{detail}")
    return 0 if not failures else 1


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""One-command offline local verifier for odoo-native-print.

Runs version/command-surface gates, shell syntax, Node tests, Odoo ESM syntax,
Python compile, XML/ADMX/ADML well-formedness, TOML/JSON syntax, GitHub Actions
workflow YAML validation (when PyYAML is installed), and ar.po msgid checks in
parallel with deterministic output order, per-step timings, and a PASS/FAIL
summary. Zero required third-party dependencies.
"""
import ast
import json
import os
import py_compile
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
import xml.etree.ElementTree as ET
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = {".git", "node_modules", "target", "__pycache__"}


def _iter_files(suffixes):
    for p in sorted(ROOT.rglob("*")):
        if p.is_file() and p.suffix in suffixes and not (SKIP_DIRS & set(p.parts)):
            yield p


def _run_cmd(cmd, env=None):
    proc = subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True)
    if proc.returncode != 0:
        out = (proc.stdout + "\n" + proc.stderr).strip()
        tail = "\n".join(out.splitlines()[-15:]) if out else f"exit {proc.returncode}"
        return False, tail
    return True, ""


def _check_py_compile():
    files = (
        sorted(ROOT.glob("odoo19/native_print/**/*.py"))
        + sorted(ROOT.glob("odoo20/native_print/**/*.py"))
        + sorted(ROOT.glob("tools/*.py"))
    )
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


def check_js_esm(text, label="<stdin>"):
    proc = subprocess.run(
        ["node", "--input-type=module", "--check"],
        input=text,
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        out = (proc.stderr or proc.stdout).strip()
        tail = "\n".join(out.splitlines()[-10:]) if out else f"exit {proc.returncode}"
        return False, f"{label}: {tail}"
    return True, ""


def _check_odoo_esm_syntax():
    files = sorted(ROOT.glob("odoo19/native_print/static/src/js/*.js")) + sorted(
        ROOT.glob("odoo20/native_print/static/src/js/*.js")
    )
    if not files:
        return False, "no Odoo JS files found"
    with ThreadPoolExecutor(max_workers=len(files)) as pool:
        results = list(
            pool.map(
                lambda f: check_js_esm(f.read_text(encoding="utf-8"), str(f.relative_to(ROOT))),
                files,
            )
        )
    failures = [err for ok, err in results if not ok]
    if failures:
        return False, "\n".join(failures)
    return True, f"{len(files)} files"


def check_yaml_text(text):
    if yaml is None:
        return True, "skipped: PyYAML not installed"
    doc = yaml.safe_load(text)
    if not isinstance(doc, dict):
        return False, f"expected YAML mapping, got {type(doc).__name__}"
    return True, ""


def _check_workflow_yaml():
    if yaml is None:
        return True, "skipped: PyYAML not installed"
    files = sorted(ROOT.glob(".github/workflows/*.yml"))
    if not files:
        return False, "no workflow YAML files found"
    for f in files:
        ok, err = check_yaml_text(f.read_text(encoding="utf-8"))
        if not ok:
            return False, f"{f.relative_to(ROOT)}: {err}"
    return True, f"{len(files)} files"


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


def _check_ar_po(rel="odoo19/native_print/i18n/ar.po"):
    po_path = ROOT / rel
    count, errors = check_po_text(po_path.read_text(encoding="utf-8"))
    if errors:
        return False, "\n".join(errors[:10])
    return True, f"{count} entries"


def _run_step(step):
    name, fn = step
    t0 = time.perf_counter()
    try:
        ok, detail = fn()
    except Exception as exc:
        ok, detail = False, f"{type(exc).__name__}: {exc}"
    return name, ok, detail, (time.perf_counter() - t0) * 1000.0


def _run_steps(steps):
    with ThreadPoolExecutor(max_workers=len(steps) or 1) as pool:
        return list(pool.map(_run_step, steps))


def _self_test():
    n, errs = check_po_text('msgid ""\nmsgstr "Header\\n"\n\nmsgid "Hello"\nmsgstr "مرحبا"\n')
    assert n == 2 and not errs, (n, errs)
    _, errs = check_po_text('msgid ""\nmsgstr ""\n\nmsgid ""\nmsgstr "bad"\n')
    assert len(errs) == 1 and "empty msgid" in errs[0], errs
    _, errs = check_po_text('msgid "Dup"\nmsgstr "1"\n\nmsgid "Dup"\nmsgstr "2"\n')
    assert len(errs) == 1 and "duplicate msgid" in errs[0], errs

    ok, err = check_js_esm("export const x = 1;\n", "ok.js")
    assert ok and not err, (ok, err)
    ok, err = check_js_esm("export const = ;\n", "bad.js")
    assert not ok and "bad.js" in err, (ok, err)

    if yaml is not None:
        ok, err = check_yaml_text("name: CI\non:\n  push:\n")
        assert ok and not err, (ok, err)
        ok, err = check_yaml_text("- not_a_mapping\n")
        assert not ok and "expected YAML mapping" in err, (ok, err)
        try:
            check_yaml_text("name: [unclosed\n")
        except yaml.YAMLError:
            pass
        else:
            raise AssertionError("expected YAMLError for malformed YAML")

    res = _run_steps([
        ("slow", lambda: (time.sleep(0.02), (True, "ok"))[1]),
        ("fast", lambda: (True, "ok")),
        ("boom", lambda: (_ for _ in ()).throw(ValueError("boom"))),
    ])
    assert [r[0] for r in res] == ["slow", "fast", "boom"], res
    assert res[0][1] is True and res[1][1] is True
    assert res[2][1] is False and "ValueError: boom" in res[2][2], res
    print("self-test OK")
    return 0


def _find_bash():
    """Return a working bash executable path, preferring Git Bash on Windows."""
    if sys.platform == "win32":
        for env_var in ("ProgramW6432", "ProgramFiles", "ProgramFiles(x86)", "LocalAppData"):
            base = os.environ.get(env_var)
            if not base:
                continue
            for rel in ("Git/bin/bash.exe", "Git/usr/bin/bash.exe", "Programs/Git/bin/bash.exe"):
                candidate = Path(base) / rel
                if candidate.is_file():
                    return str(candidate)
    return shutil.which("bash") or "bash"


def main():
    if "--self-test" in sys.argv[1:]:
        return _self_test()

    total_t0 = time.perf_counter()
    passed = 0
    failures = []

    # Isolate TMPDIR per run so concurrent runs do not collide on fixed /tmp test filenames.
    with tempfile.TemporaryDirectory(prefix="np-verify-") as tmpdir:
        sub_env = {**os.environ, "TMPDIR": tmpdir}
        steps = [
            ("versions", lambda: _run_cmd([sys.executable, "tools/check_versions.py"], sub_env)),
            ("command_surface", lambda: _run_cmd([sys.executable, "tools/check_command_surface.py"], sub_env)),
            ("bash_syntax:fetch-pdfium", lambda: _run_cmd([_find_bash(), "-n", "tools/fetch-pdfium.sh"], sub_env)),
            ("node:control-center", lambda: _run_cmd(["node", "--test", "app/tests/control-center.test.mjs"], sub_env)),
            ("node:pos_keys", lambda: _run_cmd(["node", "--test", "odoo19/native_print/tests/test_pos_keys.mjs"], sub_env)),
            ("node:live_target", lambda: _run_cmd(["node", "--test", "odoo19/native_print/tests/test_live_target.mjs"], sub_env)),
            ("node:odoo20_pos_keys", lambda: _run_cmd(["node", "--test", "odoo20/native_print/tests/test_pos_keys.mjs"], sub_env)),
            ("node:odoo20_live_target", lambda: _run_cmd(["node", "--test", "odoo20/native_print/tests/test_live_target.mjs"], sub_env)),
            ("node:odoo_esm_syntax", _check_odoo_esm_syntax),
            ("python:py_compile", _check_py_compile),
            ("xml/admx/adml", _check_xml),
            ("toml/json", _check_toml_json),
            ("yaml:workflows", _check_workflow_yaml),
            ("i18n:ar.po", _check_ar_po),
            ("i18n:odoo20_ar.po", lambda: _check_ar_po("odoo20/native_print/i18n/ar.po")),
        ]
        results = _run_steps(steps)

    for name, ok, detail, dt_ms in results:
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

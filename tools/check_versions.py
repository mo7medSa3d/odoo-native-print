#!/usr/bin/env python3
"""Version-metadata gate.

The app version lives in four files that must agree exactly, or the
installed app, its crate and its npm shell drift apart:
  - app/src-tauri/tauri.conf.json   "version"
  - app/src-tauri/Cargo.toml        version
  - app/package.json                "version"
  - app/package-lock.json           "version" (root + packages[""])
The Odoo addon versions (odoo19/native_print/__manifest__.py and
odoo20/native_print/__manifest__.py) are independent 19.0.x.y.z /
20.0.x.y.z lines; they are reported, not compared — but an unparseable
manifest is a hard failure, not a silent "?" (audit CI-L: the old gate
printed "?" and exited 0).
Also verifies the git tag (when run on a tag) matches the app version.
"""
import json
import os
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    conf = json.loads((ROOT / "app/src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
    pkg = json.loads((ROOT / "app/package.json").read_text(encoding="utf-8"))
    lock = json.loads((ROOT / "app/package-lock.json").read_text(encoding="utf-8"))
    cargo = tomllib.loads((ROOT / "app/src-tauri/Cargo.toml").read_text(encoding="utf-8"))
    android_gradle = ROOT / "android-app/app/build.gradle.kts"
    android_ver = None
    if android_gradle.exists():
        m = re.search(r'versionName\s*=\s*"([^"]+)"', android_gradle.read_text(encoding="utf-8"))
        if not m:
            sys.exit("FAIL: could not parse versionName from android-app/app/build.gradle.kts")
        android_ver = m.group(1)

    versions = {
        "tauri.conf.json": conf["version"],
        "package.json": pkg["version"],
        "package-lock.json": lock.get("version"),
        "package-lock packages['']": (lock.get("packages") or {}).get("", {}).get("version"),
        "Cargo.toml": (cargo.get("package") or {}).get("version"),
    }
    if android_ver is not None:
        versions["android-app/app/build.gradle.kts"] = android_ver
    for label, rel, prefix in (
        ("odoo19", "odoo19/native_print/__manifest__.py", "19.0."),
        ("odoo20", "odoo20/native_print/__manifest__.py", "20.0."),
    ):
        manifest_path = ROOT / rel
        if not manifest_path.is_file():
            print(f"ADDON MANIFEST MISSING ({label}): {rel}")
            return 1
        am = re.search(r'''["']version["']\s*:\s*["']([^"']+)["']''', manifest_path.read_text(encoding="utf-8"))
        if not am or not am.group(1).startswith(prefix):
            print(f"ADDON MANIFEST UNPARSEABLE ({label}): expected {prefix}* version in {rel}")
            return 1
        print(f"addon version ({label}, independent line): {am.group(1)}")
    uniq = set(versions.values())
    if len(uniq) != 1 or None in uniq:
        print(f"VERSION MISMATCH: {versions}")
        return 1
    app_version = uniq.pop()
    ref_type = os.environ.get("GITHUB_REF_TYPE", "")
    ref = os.environ.get("GITHUB_REF", "")
    tag = os.environ.get("GITHUB_REF_NAME", "")
    if not tag and ref.startswith("refs/tags/"):
        tag = ref.removeprefix("refs/tags/")
    is_tag = (ref_type == "tag") or ref.startswith("refs/tags/") or (not ref_type and not ref and tag.startswith("v"))
    if is_tag and tag.startswith("v") and tag.removeprefix("v").split("-")[0] != app_version:
        print(f"TAG MISMATCH: tag {tag} vs app version {app_version}")
        return 1
    print(f"app version consistent everywhere: {app_version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

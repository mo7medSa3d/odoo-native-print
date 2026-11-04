#!/usr/bin/env python3
"""Version-metadata gate.

The app version lives in three files that must agree exactly, or the
installed app, its crate and its npm shell drift apart:
  - app/src-tauri/tauri.conf.json   "version"
  - app/src-tauri/Cargo.toml        version
  - app/package.json                "version"
The Odoo addon version (odoo/native_print/__manifest__.py) is an
independent 19.0.x.y.z line; it is reported, not compared.
Also verifies the git tag (when run on a tag) matches the app version.
"""
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    conf = json.loads((ROOT / "app/src-tauri/tauri.conf.json").read_text())
    pkg = json.loads((ROOT / "app/package.json").read_text())
    cargo = (ROOT / "app/src-tauri/Cargo.toml").read_text()
    m = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M)
    versions = {
        "tauri.conf.json": conf["version"],
        "package.json": pkg["version"],
        "Cargo.toml": m.group(1) if m else None,
    }
    manifest = (ROOT / "odoo/native_print/__manifest__.py").read_text()
    am = re.search(r'"version":\s*"([^"]+)"', manifest)
    print(f"addon version (independent line): {am.group(1) if am else '?'}")
    uniq = set(versions.values())
    if len(uniq) != 1 or None in uniq:
        print(f"VERSION MISMATCH: {versions}")
        return 1
    app_version = uniq.pop()
    tag = os.environ.get("GITHUB_REF_NAME", "")
    if tag.startswith("v") and tag.lstrip("v").split("-")[0] != app_version:
        print(f"TAG MISMATCH: tag {tag} vs app version {app_version}")
        return 1
    print(f"app version consistent everywhere: {app_version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

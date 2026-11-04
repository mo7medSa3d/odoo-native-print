#!/usr/bin/env python3
"""Consistency gate for the Tauri command surface.

The same command set must agree in four places, or webview calls are
denied at runtime by the ACL even though the Rust handler exists:
  1. app/src-tauri/src/lib.rs        generate_handler![...]
  2. app/src-tauri/build.rs          AppManifest::new().commands(&[...])
  3. app/src-tauri/permissions/*.toml  every command granted somewhere
  4. capabilities                    local page must reach every command;
                                     the remote origin only its subset

Exits non-zero with a diff when the surfaces disagree.
Also models capabilities/toolbar.json and scans the shell's JS
surfaces for invoke() calls naming commands that do not exist.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "app" / "src-tauri"


def main():
    lib = (SRC / "src" / "lib.rs").read_text(encoding="utf-8")
    handler = set(re.findall(r"commands::([a-z_]+)", lib.split("generate_handler!")[1].split("]")[0]))
    build = (SRC / "build.rs").read_text(encoding="utf-8")
    manifest = set(re.findall(r'"([a-z_]+)"', build))
    perms = {}
    for toml in (SRC / "permissions").glob("*.toml"):
        text = toml.read_text(encoding="utf-8")
        for m in re.finditer(r'identifier = "([^"]+)".*?commands\.allow = \[(.*?)\]', text, re.S):
            perms[m.group(1)] = set(re.findall(r'"([a-z_]+)"', m.group(2)))
    granted = set().union(*perms.values()) if perms else set()
    caps = {}
    for name in ("main.json", "remote.json", "toolbar.json"):
        data = json.loads((SRC / "capabilities" / name).read_text(encoding="utf-8"))
        caps[name] = set()
        caps[name + ":groups"] = set(data["permissions"])
        for p in data["permissions"]:
            caps[name] |= perms.get(p, set())

    problems = []
    if handler != manifest:
        problems.append(f"lib.rs handler != build.rs manifest: only-handler={sorted(handler - manifest)} only-manifest={sorted(manifest - handler)}")
    if handler != granted:
        problems.append(f"handler != permissions union: ungranted={sorted(handler - granted)} unknown-grants={sorted(granted - handler)}")
    if caps["main.json"] != handler:
        problems.append(f"local capability != handler: missing={sorted(handler - caps['main.json'])}")
    remote_extra = caps["remote.json"] - handler
    if remote_extra:
        problems.append(f"remote capability grants unknown commands: {sorted(remote_extra)}")
    admin = perms.get("allow-queue-admin", set())
    leaked = caps["remote.json"] & admin
    if leaked:
        problems.append(f"remote capability leaks queue-admin commands: {sorted(leaked)}")
    # The toolbar capability (for a future child webview) may only ever
    # carry its own group; anything else appearing there is a leak the
    # old gate could not see.
    toolbar_groups = caps["toolbar.json:groups"]
    if toolbar_groups != {"allow-toolbar"}:
        problems.append(f"toolbar capability groups drifted: {sorted(toolbar_groups)}")
    toolbar_leaked = caps["toolbar.json"] & admin
    if toolbar_leaked:
        problems.append(f"toolbar capability leaks queue-admin commands: {sorted(toolbar_leaked)}")
    # Every raw invoke("...") in the shell's own JS surfaces (bridge
    # facade, Control Center, toolbar page) must name a real command.
    js_surfaces = [SRC / "src" / "lib.rs", SRC.parent / "dist" / "index.html", SRC.parent / "dist" / "toolbar.html"]
    called = set()
    for surface in js_surfaces:
        called |= set(re.findall(r'invoke\(\s*"([a-z_]+)"', surface.read_text(encoding="utf-8")))
    unknown_calls = called - handler
    if unknown_calls:
        problems.append(f"JS invokes commands missing from the handler: {sorted(unknown_calls)}")

    if problems:
        print("COMMAND SURFACE INCONSISTENT:")
        for p in problems:
            print(" -", p)
        return 1
    print(f"command surface consistent: {len(handler)} commands, remote reaches {len(caps['remote.json'])}, admin stays local")
    return 0


if __name__ == "__main__":
    sys.exit(main())

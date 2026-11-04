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
Also models capabilities/toolbar.json and scans every shell HTML
page (Control Center, boot page, toolbar page) plus the bridge in
lib.rs for invoke() calls naming commands that do not exist.

Hardened in the 1.4.8 deep-debug round (audit CI-M7): the previous
version hardcoded three capability files, scanned only two of the
three HTML pages (boot.html's invokes were invisible to it), matched
command names with [a-z_]+ (a future command containing a digit
would be silently truncated in every comparison), parsed ALL string
literals in build.rs instead of the commands array, assumed the
admin permission is literally named "allow-queue-admin" (renaming
it would have made the remote-leak check vacuous), and only saw
double-quoted invoke() calls.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "app" / "src-tauri"
NAME = r"[a-z_][a-z0-9_]*"


def main():
    lib = (SRC / "src" / "lib.rs").read_text(encoding="utf-8")
    handler = set(re.findall(rf"commands::({NAME})", lib.split("generate_handler!")[1].split("]")[0]))
    build = (SRC / "build.rs").read_text(encoding="utf-8")
    # Only the .commands(&[...]) array: any other quoted literal in
    # build.rs must not enter the comparison.
    span = build.split(".commands(&[", 1)[1].split("])", 1)[0]
    manifest = set(re.findall(rf'"({NAME})"', span))
    perms = {}
    for toml in sorted((SRC / "permissions").glob("*.toml")):
        text = toml.read_text(encoding="utf-8")
        for m in re.finditer(r'identifier = "([^"]+)".*?commands\.allow = \[(.*?)\]', text, re.S):
            perms[m.group(1)] = set(re.findall(rf'"({NAME})"', m.group(2)))
    granted = set().union(*perms.values()) if perms else set()
    caps = {}
    for capfile in sorted((SRC / "capabilities").glob("*.json")):
        data = json.loads(capfile.read_text(encoding="utf-8"))
        caps[capfile.name] = set()
        caps[capfile.name + ":groups"] = set(data["permissions"])
        for p in data["permissions"]:
            caps[capfile.name] |= perms.get(p, set())

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
    # The admin set is DERIVED, not named: it is the permission that
    # grants clear_finished_cmd (the queue-administration command).
    # If that permission disappears or is renamed, fail loudly
    # instead of leaking vacuously.
    admin = set()
    for perm_name, commands in perms.items():
        if "clear_finished_cmd" in commands:
            admin = commands
            break
    if not admin:
        problems.append("no permission grants clear_finished_cmd: cannot derive the admin set")
    leaked = caps["remote.json"] & admin
    if leaked:
        problems.append(f"remote capability leaks queue-admin commands: {sorted(leaked)}")
    # The toolbar capability (for a future child webview) may only ever
    # carry its own group; anything else appearing there is a leak the
    # old gate could not see.
    if "toolbar.json" in caps:
        toolbar_groups = caps["toolbar.json:groups"]
        if toolbar_groups != {"allow-toolbar"}:
            problems.append(f"toolbar capability groups drifted: {sorted(toolbar_groups)}")
        toolbar_leaked = caps["toolbar.json"] & admin
        if toolbar_leaked:
            problems.append(f"toolbar capability leaks queue-admin commands: {sorted(toolbar_leaked)}")
    # Every raw invoke("...") / invoke('...') in the shell's own JS
    # surfaces (bridge facade, and EVERY page in app/dist — Control
    # Center, boot page, toolbar page) must name a real command.
    js_surfaces = [SRC / "src" / "lib.rs"] + sorted((SRC.parent / "dist").glob("*.html"))
    called = set()
    for surface in js_surfaces:
        called |= set(re.findall(rf"invoke\(\s*[\"']({NAME})[\"']", surface.read_text(encoding="utf-8")))
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

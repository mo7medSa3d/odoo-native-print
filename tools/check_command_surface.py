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
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "app" / "src-tauri"


def rust_commands(text, start_marker):
    start = text.index(start_marker)
    end = text.index("]", start)
    block = text[start:end]
    return set(re.findall(r'"([a-z_]+)"', block)) or set(
        re.findall(r"commands::([a-z_]+)", block)
    )


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
    for name in ("main.json", "remote.json"):
        data = json.loads((SRC / "capabilities" / name).read_text(encoding="utf-8"))
        caps[name] = set()
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

    if problems:
        print("COMMAND SURFACE INCONSISTENT:")
        for p in problems:
            print(" -", p)
        return 1
    print(f"command surface consistent: {len(handler)} commands, remote reaches {len(caps['remote.json'])}, admin stays local")
    return 0


if __name__ == "__main__":
    sys.exit(main())

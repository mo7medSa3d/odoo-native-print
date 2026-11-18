# Worker Queue Reset & Workspace Consolidation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure the persistent worker `Queue` connection resets cleanly after any SQLite error or panic, and document the build-host-only Cargo workspace unification.

**Architecture:** Set `reset_queue = true` in `spawn_worker_loop` on `Ok(Err(e))` (when `QueueError::Db` or any queue error occurs) and `Err(_)` (panic), then sync `SOURCE_MANIFEST.txt`. Cargo workspace unification (`print-win`, `app/src-tauri`) is deferred to a host with `cargo` installed so `Cargo.lock` stays valid for `--locked` builds.

**Tech Stack:** Rust (`app/src-tauri`, `print-core`), Python (`SOURCE_MANIFEST.txt` sync).

## Global Constraints

- Minimal diff (`ponytail` discipline); do not touch `Cargo.lock` without `cargo` installed.
- `python3 tools/check_versions.py`, `python3 tools/check_command_surface.py`, and `cd app && npm test` must pass.

## Review Focus

- Worker panic or `QueueError::Db` during `worker.run_once(&printer)` must drop `worker_queue` (`reset_queue = true`) and break out of the `for printer in printers` loop so remaining iterations don't reuse a poisoned SQLite handle.

---

### Task 1: Reset `worker_queue` on `run_once` Error or Panic & Sync Manifest

**Files:**
- Modify: `app/src-tauri/src/commands.rs:3040-3057`
- Modify: `SOURCE_MANIFEST.txt`

- [ ] **Step 1:** In `app/src-tauri/src/commands.rs`, inside `match outcome`:
  - On `Ok(Err(e))`: log `e`, and if `matches!(e, print_core::QueueError::Db(_))` set `reset_queue = true; break;`.
  - On `Err(_)`: log panic, set `reset_queue = true; break;`.
- [ ] **Step 2:** Regenerate `SOURCE_MANIFEST.txt` and verify 0 mismatches.
- [ ] **Step 3:** Run `which cargo rustc`, `python3 tools/check_versions.py`, `python3 tools/check_command_surface.py`, and `cd app && npm test`.

### Task 2: Unify Cargo Workspaces & Run Full Rust/JSDOM Suite (Requires Build Host with `cargo` & Network)

**Files:**
- Modify: `Cargo.toml`, `crates/print-win/Cargo.toml`, `app/src-tauri/Cargo.toml`, `Cargo.lock`

- [ ] **Step 1:** Add `crates/print-win` and `app/src-tauri` to root `Cargo.toml` `[workspace.members]`, move shared deps into `[workspace.dependencies]`, and regenerate unified `Cargo.lock` via `cargo update --workspace`.
- [ ] **Step 2:** Run `cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` and `cd app/tests/jsdom && npm ci && npm test`.

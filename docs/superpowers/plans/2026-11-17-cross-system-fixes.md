# Cross-System Production Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve the remaining cross-system reliability, security, and performance gaps identified in the `ponytail-review` across `print-ipp`, `print-raw`, `print-core`, `app/src-tauri`, `app/dist`, and CI.

**Architecture:** Fix transport-level false failures (`print-ipp` persistent socket framing, `print-raw` `RST`/`BrokenPipe` on half-close), eliminate worker queue stalls (`Outcome::Skipped` + persistent `Queue` connection + throttled `reconcile_stale`), tighten Tauri IPC least-privilege capabilities and remove `'unsafe-inline'` from `script-src` by migrating `index.html` inline handlers to `addEventListener`, and add `jsdom` + `PDFIUM_TEST_REQUIRED=1` gates to CI.

**Tech Stack:** Rust (`print-core`, `print-ipp`, `print-raw`, `app/src-tauri`), Tauri v2, Vanilla HTML/JS, Node test runner, GitHub Actions.

## Global Constraints

- Minimal, clean diffs (`ponytail` discipline); no new external dependencies.
- Preserve all 32 guarded Tauri commands (`python3 tools/check_command_surface.py` must pass) and version parity (`python3 tools/check_versions.py` must pass).
- Preserve tolerant printer behavior: never prematurely drop or fail print jobs that were already transmitted to the printer.
- `cd app && npm test` must pass 100%.

## Review Focus

- IPP server ignores `Connection: close` and holds TCP socket open after sending `Content-Length` or `0\r\n\r\n` chunked body -> must return response immediately without waiting for `read_timeout`.
- Thermal printer sends TCP `RST` (`ConnectionReset`) or `BrokenPipe` during `shutdown(Shutdown::Write)` after `write_all` + `flush` -> must return `Ok(())`, not `SendPhaseError::Write`.
- Canceling one queued job mid-claim while another job is queued on the same printer -> worker must not sleep 2s before draining the next job.
- `settings.html` in `tab-*` still needs `app_identity` and `open_odoo_origin`, while `tab-*` must NOT have `tab_close`/`tab_switch`/`nav_*`.
- `app/dist/index.html` buttons/inputs must still work identically with `script-src 'self'` (no `'unsafe-inline'`).

---

### Task 1: IPP HTTP Body Framing Without Waiting for EOF

**Files:**
- Modify: `crates/print-ipp/src/client.rs`

- [ ] **Step 1:** Update `IppClient::post` in `crates/print-ipp/src/client.rs` so it reads incrementally up to `MAX_IPP_RESPONSE_BYTES + 1` and stops reading as soon as the HTTP headers (`\r\n\r\n`) plus either `Content-Length` bytes or the chunked terminator (`0\r\n\r\n`) are complete, instead of blocking on `read_to_end` until EOF.
- [ ] **Step 2:** Add a unit test in `crates/print-ipp/src/client.rs` using a local `TcpListener` that sends a valid HTTP `200 OK` IPP response with `Content-Length` and intentionally keeps the socket open (no `shutdown`/drop for 2s) and asserts `IppClient` returns promptly.

### Task 2: TCP 9100 `ConnectionReset`/`BrokenPipe` Tolerance on Half-Close

**Files:**
- Modify: `crates/print-raw/src/tcp9100.rs`

- [ ] **Step 1:** Update `send_raw_phased` in `crates/print-raw/src/tcp9100.rs` (`stream.shutdown(Shutdown::Write)`) to ignore `ErrorKind::NotConnected`, `ErrorKind::ConnectionReset`, and `ErrorKind::BrokenPipe` after `write_all` and `flush` have succeeded.
- [ ] **Step 2:** Add a unit test in `crates/print-raw/src/tcp9100.rs` verifying `send_raw_phased` returns `Ok(())` when the peer reads all bytes and closes/resets the socket before/during `shutdown(Shutdown::Write)`.

### Task 3: Worker `Outcome::Skipped` & Persistent `Queue` Connection

**Files:**
- Modify: `crates/print-core/src/worker.rs`
- Modify: `app/src-tauri/src/commands.rs`

- [ ] **Step 1:** Add `Outcome::Skipped` to `crates/print-core/src/worker.rs` and return `Ok(Outcome::Skipped)` when `set_state` hits `QueueError::Transition` from `JobState::Canceled`.
- [ ] **Step 2:** Update the worker loop in `app/src-tauri/src/commands.rs` so `o != Outcome::Idle` (including `Outcome::Skipped`) sets `did_work = true`, keeps a persistent `Option<Queue>` across loop iterations (reopening only on `None`/error), and throttles `reconcile_stale()` to run at most once every 15 seconds.

### Task 4: Least-Privilege `tab-*` Permission & Strict `script-src 'self'` CSP

**Files:**
- Modify: `app/src-tauri/permissions/print.toml`
- Modify: `app/src-tauri/capabilities/main.json`
- Modify: `app/src-tauri/tauri.conf.json`
- Modify: `app/dist/index.html`
- Modify: `app/tests/control-center.test.mjs`
- Modify: `tools/check_command_surface.py` (if needed for permission parity)

- [ ] **Step 1:** Add `allow-settings-nav` (containing only `app_identity` and `open_odoo_origin`) to `app/src-tauri/permissions/print.toml` and replace `allow-toolbar` with `allow-settings-nav` in `app/src-tauri/capabilities/main.json`.
- [ ] **Step 2:** Replace all inline `onclick=`/`onchange=` attributes in `app/dist/index.html` with `data-action` / `id` bindings wired via `addEventListener` in `<script>`, and update `app/tests/control-center.test.mjs` to validate `data-action`/listener bindings instead of inline attributes.
- [ ] **Step 3:** Remove `'unsafe-inline'` from `script-src` in `app/src-tauri/tauri.conf.json`.
- [ ] **Step 4:** Run `cd app && npm test` and `python3 tools/check_command_surface.py` to verify all pass.

### Task 5: CI `jsdom` + `pdfium_render` Gates & Manifest Sync

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `SOURCE_MANIFEST.txt`

- [ ] **Step 1:** Add `app/tests/jsdom` (`npm ci && npm test`) and `tools/fetch-pdfium.sh` + `PDFIUM_TEST_REQUIRED=1 cargo test -p print-render --test pdfium_render` to `.github/workflows/ci.yml` and `.github/workflows/release.yml`.
- [ ] **Step 2:** Regenerate `SOURCE_MANIFEST.txt` so all file sizes match disk 100%.

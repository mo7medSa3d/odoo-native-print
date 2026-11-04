# Verification Report — Yaseir Print Engineering Mission

Date: 2026-11-04. Environment: Linux x86_64 VM (Ubuntu 24.04.5,
root). This machine has no Windows/MSVC toolchain, no Odoo or
PostgreSQL, no Docker, and no printer hardware. Items that require
them are marked BLOCKED / NOT RUN with the exact gate that closes
them; nothing below simulates them. Earlier bundled reports
described app 1.4.2 / addon 19.0.2.8.0; this report replaces them
and records only this cycle's evidence, on the final tree
(app **1.4.12**, addon **19.0.2.15.0**).

Status key: **PASS** (ran, green), **FAIL** (ran, red — none
outstanding), **BLOCKED** (cannot run in this environment; named
gate), **NOT RUN** (not attempted / no hardware), **NOT APPLICABLE**.

## Toolchain (as run)

| Tool | Version |
|---|---|
| rustc / cargo | 1.99.1 (stable, installed via rustup 2026-11-04; clippy + rustfmt components added) |
| Node.js | v24.22.1 |
| Python | 3.12.3 |
| Tauri Linux system deps | libwebkit2gtk-4.1-dev, libsoup-3.0-dev, libjavascriptcoregtk-4.1-dev, librsvg2-dev, libssl-dev, pkg-config (installed via apt 2026-11-04; this is what makes the app crate type-checkable on Linux) |
| cargo-audit / cargo-deny | NOT RUN — `cargo-audit` could not be built in this VM (compile failed under `cargo install`; two timed-out attempts). No dependency version changed in this cycle. Gate: CI job `audit` (cargo-deny-action + rustsec/audit-check) in `.github/workflows/ci.yml`. |

## Commands executed (final tree)

```sh
# Rust workspace (crates/print-core, print-raw, print-render, print-ipp, print-tools)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Standalone crates/print-win workspace
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
# Standalone app/src-tauri workspace (Linux check; Win32 FFI bodies are cfg-gated out)
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo check --all-targets
cargo test --lib
# Gates and script suites
python3 tools/check_command_surface.py
python3 tools/check_versions.py
node odoo/native_print/tests/test_live_target.mjs
node odoo/native_print/tests/test_pos_keys.mjs
python3 -m compileall -q odoo/native_print
node --check odoo/native_print/static/src/js/*.js (handler, POS, client, systray)
# jsdom suites (after `npm install` in app/tests/jsdom)
node watchdog.test.mjs errortap.test.mjs navpill.test.mjs \
     cc-capabilities.test.mjs control-center-behavior.test.mjs
# Performance / chaos harness
cargo run --release -p print-tools -- bench
cargo run --release -p print-tools -- chaos
cargo run --release -p print-tools -- calibration
```

## Results

### Rust — PASS

| Suite | Result |
|---|---|
| `cargo fmt --all --check` (workspace) | PASS |
| `cargo fmt --all --check` (print-win) | PASS |
| `cargo fmt --all --check` (app crate) | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS, 0 errors |
| `cargo clippy` (print-win, all targets, `-D warnings`) | PASS, 0 errors |
| `cargo clippy` (app crate, all targets, `-D warnings`) | PASS, 0 errors |
| `cargo test --workspace` | **PASS — 135 passed, 0 failed.** Breakdown: print-core 70, print-ipp 7, print-raw 23, print-render 31 unit + 4 PDFium integration, print-tools 0. Baseline before this cycle's changes was 120; +15 new/changed tests (health model, stable identity, provenance, DPI, protocol parsers, durable backoff, EPL gap). |
| `cargo test` (print-win) | **PASS — 11 passed, 0 failed** (discovery/health/capability tests; Windows FFI bodies NOT compiled on Linux — see BLOCKED). |
| `cargo check --all-targets` + `cargo test --lib` (app crate, Linux) | **PASS — check clean; 5 passed, 0 failed**, including new `ipp_uri_brackets_ipv6_hosts` and `label_gap_helpers_default_to_3mm`. |

**Incident during the final run (honest record):** the first
`cargo test --workspace` on the final tree reported 2 failures in
print-core (`concurrent_same_key_enqueues_create_exactly_one_job`,
`durable_file_queue_survives_reopen`) with SQLite `DiskFull`
errors. Cause was environmental: the VM's `/tmp` tmpfs (512 MB)
was 100% full with cargo-audit build artifacts, and those two
tests use file-backed queues under `temp_dir()`. After removing
the build artifacts (510 MB free), the full suite passed
(70/70 in print-core). No code was changed to obtain the green
run. Side observation: the queue fails loudly on disk-full
(`QUEUE_DB_ERROR`-class `Db` error) rather than silently losing
data.

**Defects found and fixed by the battery itself** (all in the
final tree): stray closing brace in `printer.rs` tests; TTL-0
staleness semantics; ZPL closure lifetime; removed `Worker::sleep`
fields (print-tools + app); doc comments in parameter lists;
`too_many_arguments` fixed by introducing the `RawRender` struct
(no Clippy allow attributes were added anywhere).

### Gates and script suites — PASS

| Check | Result |
|---|---|
| `tools/check_command_surface.py` | PASS — "command surface consistent: 24 commands, remote reaches 11, admin stays local" |
| `tools/check_versions.py` | PASS — app 1.4.12 everywhere, addon 19.0.2.15.0 |
| `test_live_target.mjs` | PASS — ALL PASS |
| `test_pos_keys.mjs` | PASS — ALL PASS |
| `python3 -m compileall` (addon) | PASS |
| `node --check` (4 addon JS files) | PASS |
| XML well-formedness (6 addon XML files) | PASS |

### jsdom suites — PASS (63 checks)

| Suite | Result |
|---|---|
| `watchdog.test.mjs` | 14 pass, 0 fail |
| `errortap.test.mjs` | 7 pass, 0 fail |
| `navpill.test.mjs` | 14 pass, 0 fail |
| `cc-capabilities.test.mjs` | 14 pass, 0 fail |
| `control-center-behavior.test.mjs` | 14 pass, 0 fail |

### Performance / chaos harness — PASS (measured on this VM)

Run with `cargo run --release -p print-tools`. These are
software-pipeline numbers (in-memory/mock backends), not printer
evidence, and are not comparable to the 2026-11-02 results
(different machine and harness configuration — the chaos harness
now runs with zero backoff by design; see the code comment).

| Harness | Result |
|---|---|
| `bench` | PASS — 1000 jobs, enqueue 137 ms, process 218 ms, **4587 jobs/sec** sustained, burst 200 jobs in 13 ms, no loss (1000/1000), no duplicates |
| `chaos` | PASS — 300 jobs, 11 simulated restarts, done=300, dead_letter=0, failed=0, recovered_requeued=0, ambiguous=0, no loss (300/300), no duplicate success |
| `calibration` | PASS — calibration PDF 4511 bytes, 1 page, 210.0×297.0 mm; thermal bitmap 576×639 dots, 46032 ESC/POS bytes |

No criterion benchmarks ship in this repo; no percentage
improvements are claimed. Startup time, WebView2 startup, and
discovery latency were not measured (no Windows/WebView2 here).

### Odoo — BLOCKED

| Check | Status | Gate |
|---|---|---|
| Module install / upgrade on fresh + existing DB | BLOCKED | `odoo.yml` CI (Docker `odoo:19.0` + PostgreSQL) |
| Python test suite incl. new `test_job_status_retry_after_failure_converges` | BLOCKED | `odoo.yml` CI. Local evidence: `py_compile` passes; the test exercises only the changed controller paths |
| Access rights / record rules / multi-company | BLOCKED | `odoo.yml` CI |
| Report + POS flows against real Odoo 19 | BLOCKED | Owner field test / `odoo.yml` |
| Asset-bundle compilation (libsass) | BLOCKED | Live Odoo bundle fetch (release gate per PROGRESS.md) |
| Fuzz targets (`fuzz/ipp_response`, `pdf_info`, `png_decode`) | NOT RUN | `cargo fuzz` (not executed this cycle) |

### Windows — BLOCKED

| Check | Status | Gate |
|---|---|---|
| Full Tauri app compilation (MSVC) | BLOCKED | CI `windows` job |
| `print-win` FFI bodies (`win.rs`: `printer_capabilities`, `printer_health`, GDI/RAW paths) | BLOCKED | CI `windows` job — these files are `cfg(windows)`-gated and were **not** compiled on Linux; only the non-Windows stubs and the platform-free discovery code were compiled here |
| MSI / NSIS installer build, PDFium bundling on Windows | BLOCKED | CI `windows-installers-unsigned` job |
| `npwin_probe` on Windows | BLOCKED | CI `windows` job / printer-lab runner |
| WebView2 behavior | BLOCKED | Windows CI / manual |

### Hardware — NOT RUN

Every row of `docs/hardware-test-matrix.md` remains unchecked: no
physical printer of any class was available. Protocol parsers are
unit-tested against documented byte layouts only (see
`PRINTER_COMPATIBILITY_MATRIX.md` for sources and the
library-only status). No simulated result is presented as a
hardware PASS anywhere in this delivery.

## Summary counts

- Rust tests run: **151 passed, 0 failed** (135 workspace + 11 print-win + 5 app).
- jsdom checks: **63 passed, 0 failed.**
- Node suites: **2 passed.**
- Gates: **2 passed.**
- Harness: **bench + chaos + calibration PASS.**
- Failed: **0 outstanding** (the 2 transient DiskFull failures
  were environmental; see above).
- Skipped: Odoo suite, Windows build, hardware matrix,
  dependency audit, fuzz targets — all labeled above with gates.

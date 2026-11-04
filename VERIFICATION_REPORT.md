# Verification Report — Yaseir Print Engineering Mission

Date: 2026-11-04. Environment: Linux x86_64 VM (this build machine has
no Windows, no WebView2, and no printer hardware — items that require
them are marked BLOCKED/NOT RUN with the exact gate that closes them;
nothing below simulates them).

## Toolchain (as run)

- rustc / cargo 1.99.1
- Node v24.22.1
- Python 3.12.3
- Tauri 2.12.5 (pinned by `app/src-tauri/Cargo.lock`)
- rusqlite 0.32.1
- Odoo 19 (source tree in the verification environment) + PostgreSQL 16

## Intake verification

- Supplied ZIP SHA-256:
  `fb9ce79013a82ce43194ef70c46f3f7e69a1953ee79b712240bf838c80a0fbca`
- Extracted without errors (149 files); `diff -rq` against the known
  v1.4.1 tree: **zero differences** — the audit started from exactly
  the delivered v1.4.1 / addon 19.0.2.7.0 content.

## Baseline (before fixes, on the extracted tree)

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets` | PASS (clean) |
| `cargo test --workspace` | 102 passed, 0 failed |
| `tools/check_command_surface.py` | PASS (23 commands, remote 13) |
| `tools/check_versions.py` | PASS (app 1.4.1) |
| Node: live-target + POS key suites | PASS |
| Odoo scoped suite (fresh DB) | 15 passed, 0 failed |

## Final results (after fixes, versions: app 1.4.2, addon 19.0.2.8.0)

### Rust

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` (workspace) | PASS |
| `cargo fmt -- --check` (print-win, standalone) | PASS |
| `cargo fmt -- --check` (app crate, standalone) | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo clippy --all-targets -- -D warnings` (print-win) | PASS |
| `cargo test --workspace` | **110 passed, 0 failed** (baseline 102; +8 net new tests) |
| `cargo test` (print-win standalone) | **10 passed, 0 failed** (baseline 9; +1) |
| print-core suite | 54 passed (baseline 50) |
| print-raw suite | 18 passed (baseline 15) |
| print-render suite | 28 passed |
| print-ipp suite | 6 passed (one test rewritten: it asserted the defective mislabeled-format behavior; see changelog) |

New regression tests added: status-decoder intervention flags; local
port classification; retry attempts monotonicity; reconcile never
fails a `printing` job; DLE EOT 4 paper bit-table; DLE EOT frame
validation; TSPL control-injection stripping; IPP blocking reasons
(incl. "warnings do not block"); IPP parameter/case format matching;
discovery cache TTL/single-refresh; device-registry read-only ACL
(Odoo). The `set_state` atomic guard and payload-conflict comparison
are covered by the existing queue suites' transition/duplicate tests
plus code-level review (a deterministic two-connection race test is
not part of the suite; the guarded UPDATE pattern mirrors the already
tested `claim_next`).

### Consistency gates

| Gate | Result |
|---|---|
| `tools/check_command_surface.py` (extended: toolbar capability modeled, JS invoke scan) | PASS — 23 commands, remote reaches 13, admin stays local |
| `tools/check_versions.py` | PASS — app 1.4.2 everywhere; addon 19.0.2.8.0 |

The extended gate caught one real defect on its first run
(`open_odoo_origin` in two permission groups), which was fixed; the
gate then passed.

### JavaScript / bridge / Control Center (jsdom harnesses)

| Suite | Result |
|---|---|
| Control Center DOM suite | PASS (exit 0) |
| Control Center behavior suite | 14 passed, 0 failed |
| Bridge watchdog scenarios | 7 passed, 0 failed |
| Bridge error-tap scenarios | 7 passed, 0 failed |
| `node --check` on all addon JS + Control Center extraction | PASS |

### Node protocol/logic suites (addon)

| Suite | Result |
|---|---|
| `test_live_target.mjs` (target resolution incl. prefill priority) | ALL PASS |
| `test_pos_keys.mjs` (POS idempotency keys) | ALL PASS |

### Odoo 19 (PostgreSQL 16, scoped suite `--test-tags /native_print`)

| Scenario | Result |
|---|---|
| Fresh install, addon 19.0.2.8.0 | **16 passed, 0 failed, 0 errors** |
| Upgrade 19.0.2.7.0 → 19.0.2.8.0 (install from the pre-mission tree, then `-u native_print` with tests) | **16 passed, 0 failed, 0 errors** |

(15 pre-existing tests + 1 new read-only-registry ACL test.)

## Performance notes (measured, not projected)

- No benchmark suite ships in this repository (no criterion benches);
  no percentage improvements are claimed. The one structural
  performance fix is the print-win discovery snapshot cache: the
  worker previously paid a full spooler enumeration **per job**
  (enumeration over offline network queues is documented in-code as
  taking seconds); it now shares one enumeration per 5 s window
  across callers, single-flight. Its behavior is pinned by the
  `discovery_cache_shares_snapshot_within_ttl` unit test (two calls,
  one enumeration; expiry refreshes).
- Observed wall times in this VM (environment timings, not product
  benchmarks): full workspace test run ≈ 25 s cold / ≈ 4 s warm;
  Odoo scoped suite ≈ 40 s per scenario.
- Submission-path latency: `print_pdf` / `print_image` moved off the
  Tauri main thread (async commands), eliminating the UI freeze
  during PDFium parsing of large documents; not separately
  benchmarked.

## Fault-injection coverage (automated, in-suite)

Exercised by existing + new tests: duplicate idempotency keys
(incl. same-size different payload), queue caps/backpressure,
crash-recovery classification (ambiguous jobs are never auto-reprinted),
stale-job reconciliation boundaries, out-of-order/duplicate status
posts (Odoo), terminal-state finality, watcher poll-error tolerance,
unreachable TCP printer fast error, malformed IPP responses, PDF/PNG
limit enforcement. Physical fault injection (power-off mid-job, paper
removal, cover open) requires hardware — see below.

## BLOCKED / NOT RUN (with the gate that closes each)

| Item | Status | Gate |
|---|---|---|
| Windows/MSVC compilation of the app crate + print-win FFI changes | BLOCKED in this environment (no Windows toolchain; proven repeatedly) | Upload the delivered ZIP to GitHub `main`; CI jobs `windows` (tests) and the installer job must go green — the same gate that verified every prior release. App-crate edits in this mission are pattern-identical to code already in the same files (documented per-change in the changelog). |
| MSI/NSIS installer build + artifact | BLOCKED (follows the Windows build) | Same CI run; expect the `windows-installers-unsigned` artifact (~18 MB) as in runs #71/#75. |
| PDFium bundling on Windows | NOT RUN | Same CI/installer run + first-launch smoke test on a Windows machine. |
| All physical printer tests (thermal USB/network, Zebra, TSC, laser, inkjet, IPP hardware, virtual-printer output) | NOT RUN — no printer hardware exists in this environment | `docs/hardware-test-matrix.md` on the owner's Windows machine / printer-lab runner. Every cell of that matrix remains honestly unchecked. |
| POS + report printing against the production Odoo (68.221.27.248) | NOT RUN | Owner field test after installing the CI build + upgrading the addon to 19.0.2.8.0. |
| Soak / 72 h stability | NOT RUN | Owner pilot phase (pre-existing gate). |

## ZIP validation (final deliverable)

Performed after packaging (see the delivery message for the checksum):

1. `unzip -t` integrity check on the final archive.
2. Extraction into a brand-new clean directory.
3. From the extracted copy: `cargo fmt --check`, both consistency
   gates, `cargo test --workspace`, Node suites — all re-run and
   passing (results identical to the working-tree runs above).
4. Archive listing inspected: no `target/`, `node_modules/`,
   `__pycache__`, `.git`, secrets, or machine-local files; lockfiles,
   PDFium resources/notices, tests, workflows, and docs present.

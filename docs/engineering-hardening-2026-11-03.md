# Engineering Hardening — 2026-11-03 (app v1.3.0 / addon 19.0.2.3.0)

Execution record for the owner's 21-point repair plan (four phases).
Every item is either implemented with test evidence in this tree, or
marked BLOCKED with the exact unblocking procedure. Nothing here is
claimed on the strength of intention.

## Phase 1 — Correctness & Security

1. **build.rs ↔ lib.rs command surface — FIXED + GATED.** The v1.2.0
   commands (`report_environment`, `clear_web_data_cmd`) were missing
   from `build.rs`'s AppManifest and from every permission group:
   Tauri's ACL would have denied them at runtime despite the Rust
   handlers existing. Also found: `open_control_center` was
   local-only in the ACL but is invoked from Odoo pages (systray /
   Ctrl+Shift+P / watchdog) — it would have been denied too. Now:
   build.rs lists all 19 commands; `report_environment` joined
   allow-print-status; a new `allow-app-shell` group
   (open_control_center, clear_web_data_cmd) is granted to both
   capabilities; queue-admin stays local-only.
   `tools/check_command_surface.py` proves the four surfaces agree
   (19 commands; remote reaches 13; admin local) and runs in CI and
   the release gate.
2. **Validation before parsing — FIXED.** `print_pdf` parsed the PDF
   (PDFium) and `print_image` decoded the PNG *before* any limit
   check. Both now run `validate_payload_limits` (size, copies) and
   the idempotency-key check first; the full validation (pages,
   dimensions) still runs after parsing.
3. **Atomic enqueue — FIXED + PROVEN.** New
   `Queue::enqueue_with_payload` writes job + payload + options in a
   single SQLite transaction; all submit paths (PDF, text, image,
   test print) use it. A two-connection race test on the same
   database file proves exactly one winner and one job for a shared
   key.
4. **Odoo status state machine — FIXED + TESTED.** `job_status` now
   enforces `ALLOWED_TRANSITIONS` (forward-only, skips legal,
   no cancel from printing, terminal finality kept, attempts
   monotonic). Illegal posts are answered
   `ignored: "illegal_transition"` with the recorded state. New
   HttpCase test covers backward moves, cancel-from-printing,
   same-state refresh and attempt regression; scoped suite 8/8.
5. **HTTP/HTTPS policy — HARDENED (policy unchanged by owner
   decision).** The bridge now shows a per-session unencrypted-
   connection strip on every HTTP page in the app. The policy itself
   (HTTPS any host; HTTP any IP) remains the owner's recorded
   decision in ADR-0002; the Control Center warning stays.
6. **Diagnostics privacy + CSP — DONE.** The bundle is metadata-only
   by construction (audited field by field; documented in
   runbooks). The local app now ships a real CSP via
   `tauri.conf.json` (`default-src 'self'`, IPC connect only).

## Phase 2 — Reliability

7. **POS monitoring timeout — FIXED.** `watchPosJob` now uses the
   report ladder (~5 minutes, 600 ms → 2 s polls) instead of ~3.
8. **POS idempotency — FIXED + TESTED.** Receipt keys are
   attempt-based per order per day (`pos-<ref>-<day>-a<n>`): a
   resubmission within 15 s reuses the in-flight key (desktop
   dedupes — no double receipt), and the watcher tombstones the
   attempt on terminal state so a deliberate reprint mints the next
   attempt. (The tombstone matters: clearing the record reset the
   counter and reissued the same key — caught by the new node test
   `tests/test_pos_keys.mjs`.) Report submissions gained an
   in-flight guard keyed by report + records: a double-click cannot
   print twice.
9. **Global queue limits — DONE.** 2000 active jobs globally (plus
   the existing 500 per printer), enforced inside the enqueue
   transaction; duplicates of an existing key never count as load.
10. **Reconciliation — DONE + TESTED.** `jobs.updated_at` (in-place
    migration) + `Queue::reconcile_stale`: working-state jobs older
    than 900 s are failed with `RECONCILED_STALE`; the app worker
    loop runs it every pass. Queued jobs and pre-migration rows are
    never touched.
11. **Crash-recovery / disk-failure tests — DONE (queue level).**
    New tests: atomic enqueue completeness, concurrent-enqueue
    race, global-limit refusal, stale reconciliation, diag-id
    stability — on top of the pre-existing recovery-semantics and
    chaos suites. (True disk-full injection remains a printer-lab
    item; SQLite WAL + single-transaction writes are the defence.)

## Phase 3 — Compatibility & Quality

12. **Odoo 19 tests — RUN.** Scoped addon suite 8/8 green on a fresh
    DB. Full dependency-closure run (base + web + native_print,
    untagged, fresh DB): **1567 tests, 2 failed + 2 errors — all four
    in Odoo's own suites, none in native_print**: base
    test_configmanager (2, config-file handling in this VM) and
    auth_passkey demo tests (2, WebAuthn needs a browser
    environment). All 8 native_print tests pass inside the full run.
    (The duplicate-key ERROR lines in the log are the expected,
    asserted constraint rejection inside the duplicate-job test.)
13. **POS receipt rendering tests — DONE.** New print-render tests:
    tall receipts band completely (bands sum to the full height,
    feed + cut at the end), wide receipts downscale proportionally to
    the roll, the 112 mm head maps to 832 dots, and invalid target
    widths are rejected.
14. **Windows installer on a clean machine — BLOCKED (no Windows
    here).** Procedure + matrix: `docs/hardware-test-matrix.md`
    (I1–I6), executed from the CI installers artifact.
15. **Hardware test matrix — DOCUMENTED, execution BLOCKED (no
    printers here).** `docs/hardware-test-matrix.md` defines every
    cell (printer class × mode × paper × case) and how to run it
    with `npwin_probe` on the printer-lab runner.
16. **Documentation & version metadata — DONE.** PROGRESS,
    api-contract (bridge additions, state machine, queue
    guarantees), runbooks (privacy), this report; app version 1.3.0
    aligned in all three files and gated.

## Phase 4 — Product Engineering

17. **Structured logging + diagnostics IDs — DONE.** A durable
    `job_events` table records enqueue/state/recovery/reconcile
    events; every job has a stable diagnostics id
    (`diag_id_for_key`, `np-<16 hex>`) shown in job summaries and the
    diagnostics bundle, whose new `recent_events` section carries
    the last 100 events (metadata only).
18. **Error catalog — DONE.** `docs/error-catalog.md`: every code
    the product surfaces, by layer, with meaning and the user's next
    action. New codes from this pass (RECONCILED_STALE, QueueFull)
    are included.
19. **Release automation — HARDENED.** CI and the release gate now
    run both consistency gates (command surface, versions) and the
    addon JS unit tests; the release gate also checks the tag
    against the app version.
20. **Code signing — WIRED, BLOCKED on the certificate.** tauri.conf
    carries the signing fields (sha256 + timestamp); the release
    workflow imports the certificate from the
    `WINDOWS_CERTIFICATE_*` secrets and injects the thumbprint when
    present, and logs an explicit UNSIGNED notice when absent.
    Procedure: `docs/release-checklist.md`. Only the purchase is
    outstanding.
21. **Compatibility policy — DONE.** `docs/compatibility.md`:
    supported matrix (Odoo 19, Windows 10/11, evergreen WebView2),
    API-v1 pairing rules with graceful degradation both ways,
    versioning, upgrade/downgrade policy, deprecation rule.

## Verification in this environment

- print-core: 50 tests pass (6 new), clippy `-D warnings` clean.
- print-render: 28 tests pass (3 new receipt tests).
- Odoo scoped suite: 8/8 on a fresh DB (new state-machine test
  included).
- Bridge jsdom: 7/7 (watchdog, HTTP strip incl. dismiss, healthy and
  login pages untouched).
- Control Center node tests: 9/9.
- Gates: command surface consistent; versions consistent (1.3.0).
- The app crate's Windows compile remains the CI gate on push
  (no Windows/GTK in this VM); the printer-lab matrix remains the
  hardware gate.

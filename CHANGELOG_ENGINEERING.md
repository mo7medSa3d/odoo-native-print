# Engineering Changelog — Audit Repair Cycle (2026-11-04)

Versions: app **1.4.11 → 1.4.12**, addon **19.0.2.14.0 → 19.0.2.15.0**.
Finding IDs refer to `ENGINEERING_AUDIT.md` §3. Unless stated, each
change ships with the regression tests listed in
`VERIFICATION_REPORT.md`. No model, migration, permission, capability,
workflow, or translation file was changed; the version bump needs no
Odoo migration script.

## C1 — Durable retry backoff (queue/worker isolation)

- **Why:** the worker thread slept through retry backoff (up to
  30 s), stalling every printer behind one offline printer.
- **Root cause:** backoff implemented as a thread sleep in
  `Worker::run_once` instead of queue state.
- **Fix:** `jobs.available_at` column + `idx_jobs_claim_available`
  index (schema user_version 2, idempotent migration);
  `claim_next` filters on `available_at`; new transactional
  `Queue::requeue_with_delay`; `retry_job` clears the delay for
  queued jobs too; `Worker` sleeper parameter removed; app worker
  loop and chaos harness updated.
- **Files:** `crates/print-core/src/{queue,worker}.rs`,
  `crates/tools/src/main.rs`, `app/src-tauri/src/commands.rs`
  (worker construction only).
- **Tests:** `worker::tests::retry_then_dead_letter` (backoff
  unclaimable, manual retry clears delay, then dead-letter);
  chaos harness reconfigured to zero backoff with a comment
  (simulated restarts are near-instant; backoff durability is
  covered by the unit test).
- **Regression risk:** low. Claim semantics change only for jobs
  inside a backoff window; recovery paths clear delays for
  in-flight rows only. A job that previously retried after a
  sleep now retries after the same delay measured in wall time.
- **Compatibility:** existing databases migrate automatically;
  old rows get `available_at = 0` (immediately claimable).

## C2 — Retryable outcomes in Odoo sync (state machine)

- **Why:** jobs that failed once then printed stayed `failed` in
  Odoo forever; the retry's outcome post was discarded.
- **Root cause:** the controller treated `failed`/`dead_letter`
  as terminal while the desktop machine allows re-entry; the JS
  watchers stopped at the first failure.
- **Fix:** `TERMINAL_STATES = {done, canceled}`; `failed` and
  `dead_letter` accept `queued` re-entry (+ forward skips) in
  `ALLOWED_TRANSITIONS`; report watcher continues past retryable
  outcomes (notify once per attempt); POS watcher releases the
  order on first failure but keeps a background poll with
  once-per-outcome posting and never touches the order again.
- **Files:** `odoo/native_print/controllers/main.py`,
  `odoo/native_print/static/src/js/{native_print_handler,pos_native_print}.js`,
  `odoo/native_print/tests/test_controllers.py` (new retry test).
- **Tests:** new `test_job_status_retry_after_failure_converges`
  (Odoo suite BLOCKED here — Python-compiled only, must run in
  `odoo.yml` CI); Node live-target/POS-key suites still pass.
- **Regression risk:** low–medium. Posts that were previously
  discarded as `terminal_state_is_final` are now accepted for
  retryable states only; `done`/`canceled` finality is unchanged
  and covered by the existing lifecycle test. A stale duplicate
  `failed` post after a retry's `done` is still discarded.
- **Compatibility:** the `/native_print/job_status` wire contract
  is unchanged; older desktop builds posting the same states see
  identical behavior except retry outcomes now land.

## C3 — Multi-condition printer health model

- **Why:** simultaneous conditions were lost; stale `Ready` could
  be displayed as live; the pre-print check was fail-open.
- **Root cause:** single-value status enum with no timestamp,
  TTL, source, or condition set.
- **Fix:** new `health.rs` (`PrinterHealth`, `HealthCondition`,
  `HealthSource`, TTL-0-always-stale); spooler/IPP/TSPL/ESC-POS
  decoders; `printer_health()` in print-win (error on failed
  read, never fabricated Ready); app gate uses
  `blocks_printing()`; Control Center shows conditions + source
  + STALE marker; re-render signature includes the health
  fingerprint. `PAPER_PROBLEM` maps to `UserIntervention` in the
  health decoder (legacy single-status decoder mapped it to
  `Error`); both block printing.
- **Files:** `crates/print-core/src/{health,lib,printer}.rs`,
  `crates/print-win/src/{discovery,win,lib}.rs`,
  `app/src-tauri/src/commands.rs`, `app/dist/index.html`.
- **Tests:** health unit tests (simultaneous spooler conditions,
  work-offline, IPP warnings vs blockers, TSPL table, ESC/POS
  combination, staleness incl. TTL 0, serde round-trip);
  `convert_populates_health_and_stable_id`; jsdom suites pass
  with the new card markup.
- **Regression risk:** low. `primary_status()` preserves the old
  single-status semantics for existing consumers; the gate blocks
  on a superset of the old conditions only where multiple
  conditions were previously hidden.
- **Compatibility:** `PrinterInfo` gains `stable_id`/`health`
  with serde defaults; older consumers ignore the fields.

## C4 — Capability provenance

- **Why:** driver query failures were presented as concrete
  capability facts.
- **Root cause:** `DeviceCapabilitiesW` −1 clamped into defaults
  with no provenance tracking.
- **Fix:** `CapabilityProvenance` per field +
  `from_driver_report`; `printer_capabilities` passes `None`
  for failed queries; legacy payloads deserialize all-`Unknown`.
- **Files:** `crates/print-core/src/printer.rs`,
  `crates/print-win/src/win.rs`.
- **Tests:** provenance flagging + legacy deserialization;
  DPI-preference tests.
- **Regression risk:** low. Fallback values are unchanged; only
  their provenance labeling is new.
- **Compatibility:** serialized `PrinterCapabilities` gains a
  `provenance` field with a serde default.

## C5 — Stable printer identity

- **Why:** display names are not identities; renames and shared
  ports split/merged devices incorrectly.
- **Root cause:** no identity field existed.
- **Fix:** `stable_printer_id` (FNV-1a over normalized driver +
  uppercase port, endpoint ports only) + `same_physical_endpoint`;
  populated by discovery; surfaced in Control Center signatures.
- **Files:** `crates/print-core/src/printer.rs`,
  `crates/print-win/src/discovery.rs`.
- **Tests:** rename/shared-port/virtual-port identity cases.
- **Regression risk:** low. Identity is additive; no merging
  behavior was introduced.
- **Compatibility:** does not survive driver reinstall / USB
  renumbering (documented limitation).

## C6 — Device-DPI rendering and label gap threading

- **Why:** 300-dpi devices received 203-dpi rasters; label gaps
  were hardcoded and unconfigurable.
- **Root cause:** literal `203` in dispatch and literal gaps at
  every label call site; no gap field in `JobOptions`.
- **Fix:** `preferred_raw_render_dpi` + `render_dpi` threaded
  through one capabilities read (label box, PDF render, TSPL mm
  conversion, ESC/POS head-dot scaling); `RawRender` parameter
  struct (Clippy `too_many_arguments`); `label_gap_mm` from
  request → queue JSON → TSPL `GAP` / EPL `Q` gap;
  `epl_label_copies_with_gap` added (old entry point delegates
  with 24 dots).
- **Files:** `crates/print-core/src/{printer,queue}.rs`,
  `crates/print-raw/src/epl.rs`,
  `app/src-tauri/src/commands.rs`.
- **Tests:** DPI preference, EPL gap threading, label-gap
  helpers (3 mm → 24 dots at 203 dpi, 35 at 300 dpi), app lib
  tests pass.
- **Regression risk:** low–medium. Rendering resolution now
  follows driver-reported values; a driver that misreports
  resolutions changes output scale. The 203 fallback applies
  whenever no resolutions are reported (the common case today),
  preserving existing output.
- **Compatibility:** `label_gap_mm` is optional on the wire
  (serde default); older callers get the 3 mm default.

## C7 — IPP IPv6 printer URIs

- **Why:** IPv6 IPP printers received a malformed `printer-uri`.
- **Root cause:** brackets stripped by `parse_host_port` were
  never restored when building the URI.
- **Fix:** `NativePrintBackend::ipp_uri` brackets `:`-containing
  hosts; round-trip unit tests.
- **Files:** `app/src-tauri/src/commands.rs`.
- **Tests:** `ipp_uri_brackets_ipv6_hosts`.
- **Regression risk:** none for IPv4/hostnames (byte-identical
  output); fixes IPv6 only.

## C8 — Protocol status parsers (library only)

- **Why:** no TSPL/ZPL status parsing existed; DLE EOT coverage
  was n=2/n=4 only, unwired.
- **Root cause:** never implemented; status came only from the
  spooler.
- **Fix:** DLE EOT n=1/n=3 queries + decoders (incl. n=2
  documentation correction: no cover bit); TSPL `ESC !?`
  decoder; ZPL `~HS`/SGD parser (both framings, empty-tolerant);
  all table-tested, exported from `print-raw`, deliberately NOT
  wired into print paths.
- **Files:** `crates/print-raw/src/{tcp9100,tspl,zpl,lib}.rs`.
- **Tests:** `dle_eot_error_table_n1_n3`,
  `tspl_status_byte_table`, ZPL framing/short/empty tests.
- **Regression risk:** none — new code paths only, no existing
  call sites changed.
- **Compatibility:** additive API only.

## Incidental fixes found by the battery (no behavior change intended)

- `printer.rs` test module had a stray closing brace left by an
  edit (compile error); removed.
- `PrinterHealth::is_stale` treated TTL-0 observations as fresh
  for past timestamps; TTL 0 is now always stale.
- `zpl.rs` field-access closure had an unnameable lifetime;
  replaced with a nested function.
- `print-tools` and app worker constructions still passed the
  removed `Worker::sleep` field; removed.
- Doc comments inside function parameter lists (rejected by
  rustc); converted to line comments.
- `cargo fmt` normalization in `crates/print-win` and the app
  crate (formatting only).

## Documentation

- `ENGINEERING_AUDIT.md`, `VERIFICATION_REPORT.md`,
  `CHANGELOG_ENGINEERING.md` rewritten from the current code and
  this cycle's actual test evidence (the bundled copies described
  1.4.2 / 19.0.2.8.0).
- `PRINTER_COMPATIBILITY_MATRIX.md` updated: DLE EOT n=1/n=3,
  TSPL/ZPL parsers as library-only, EPL/TSPL gap threading,
  IPP `is_blocked()` wiring (was already live since 1.4.6),
  health model + staleness, DPI scaling, unknown-paper refusal.
- `README.md` rewritten to the actual architecture and versions
  (was v1.0.0 / addon 19.0.1.2.0-era, incl. an incorrect IPP
  format-negotiation claim).
- `PROGRESS.md`: 1.4.12 entry appended; header stamp updated.

## Not changed (by design)

Licensing/activation, code signing, origin policy (ADR-0002),
permissions/capabilities, workflows, migrations, translations,
and the HTTP warning/strip behavior. No dependency version was
changed in this cycle (see verification report for the audit
status).

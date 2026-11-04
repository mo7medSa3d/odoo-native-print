# Engineering Changelog — Audit Repair Cycle

Versions: app **1.4.1 → 1.4.2**, addon **19.0.2.7.0 → 19.0.2.8.0**.
Finding IDs refer to `ENGINEERING_AUDIT.md` / the audit tracks
(A = Rust core, B = app shell, C = Odoo addon, D = protocols/status).
Unless stated, each change ships with the regression tests listed in
`VERIFICATION_REPORT.md`.

## print-core

- **Atomic `set_state`** (A-M3). The state write is guarded on the
  previously read state (`WHERE id=? AND state=<old>`); 0 rows changed
  returns the transition error instead of silently winning a race.
  Risk: low — mirrors the proven `claim_next` guard.
- **Retry keeps attempts** (A-M1). `retry_job` no longer resets
  `attempts=0`; monotonicity now holds across manual retries.
- **Reconcile excludes `printing`** (A-M2). `reconcile_stale` covers
  `rendering`/`spooling` only; long physical prints are no longer
  force-failed, and the worker's final transition can no longer
  collide with a reconciliation write.
- **Duplicate-conflict detection compares payload bytes** (A-L1), not
  just size. First writer still wins; only the diagnostic log is
  affected.
- **Unknown DB state strings are an error** (A-L4) instead of a silent
  coercion to `failed` in `row_to_job`.
- **`PRAGMA user_version` = 1 marker** (A-L5) set on first open so
  future migrations key off a version instead of ad-hoc probes.
  Non-destructive; existing databases are untouched apart from the
  marker.
- **Status decoder completeness** (D-2/A-L10). New `UserIntervention`
  status; USER_INTERVENTION / MANUAL_FEED / OUTPUT_BIN_FULL map to it
  (and to `needs_intervention`), PAPER_PROBLEM → `Error`,
  NOT_AVAILABLE → `Offline`, BUSY → `Printing`. Serde adds one
  snake_case value (`user_intervention`); JS consumers display status
  strings they do not recognize without breaking.
- **Port classification** (A-L11). USB/COM/LPT/FILE ports classify as
  `Local` before the network heuristics; IPv4 check validates octet
  ranges.
- **Deterministic quirk lookup** (A-M4). Longest matching driver key
  wins, independent of HashMap iteration order.

## print-render

- **PDFium lock resilience** (A-M10). All lock acquisitions recover a
  poisoned mutex instead of panicking; the one in-lock `.expect` on
  the grayscale conversion became a per-job `BitmapFailed` error.

## print-raw

- **DLE EOT 4 paper status** (D-5). New `query_paper_status` +
  `decode_dle_eot_paper` (near-end = bits 2–3 pair, paper-end = bits
  5–6 pair) with a bit-table test; the n=2 comment that misattributed
  paper bits was corrected.
- **Status frame validation** (A-L13). DLE EOT responses are checked
  against the frame's fixed bits; up to 4 bytes are skipped looking
  for a valid frame (ASB/noise tolerance); otherwise an `InvalidData`
  error. Applies to both status queries.
- **TSPL TEXT sanitization** (A-L14). Control characters are stripped
  (CR/LF could inject arbitrary TSPL commands); quote folding kept.

## print-ipp

- **PDF-only format promise** (D-1). `FORMAT_PREFERENCE` is now
  `["application/pdf"]`; raster-only printers get `NoUsableFormat`.
  The pre-existing test that asserted the old mislabeling behavior was
  rewritten to assert the honest contract (renamed
  `format_choice_without_pdf_is_rejected_not_mislabeled`) — the old
  expectation encoded the defect.
- **Status interpretation** (D-8). Requests now include
  `printer-is-accepting-jobs`; `PrinterCaps.accepting_jobs` added;
  `blocking_reasons()` / `is_blocked()` interpret the documented
  reason set with RFC 8011 severity suffixes stripped; warnings
  (toner-low, media-low) do not block. **Remaining step** (documented,
  not applied): the app dispatch should call `is_blocked()` where it
  currently calls `is_stopped()` — a one-token change in
  `app/src-tauri/src/commands.rs` that must ride the Windows CI build
  rather than ship unverified from this environment.
- **Lenient format matching** (A-L16). Comparisons use the MIME base
  type (parameters/case ignored).

## print-win (Windows FFI — verified by Linux-side tests + Windows CI gate)

- **RAW write loop + abort** (H-1/D-3). `print_raw` loops
  `WritePrinter` until the payload is fully spooled; on failure it
  calls the newly declared `AbortPrinter` instead of ending the job;
  `GetLastError` is captured immediately after the failing call
  (including the `StartPagePrinter` branch); chunk length uses a
  checked u32 conversion. **This code compiles only on Windows**; it
  follows the file's existing FFI patterns exactly and is gated on CI.
- **Discovery snapshot cache** (B-5). New platform-free
  `DiscoveryCache` (single-flight, TTL) in `discovery.rs`, used by the
  Windows `discover_printers` with a 5 s TTL; the uncached body is
  unchanged. Unit-tested on Linux.
- **EnumPrinters probe errors propagate** (A-L8) instead of reading as
  an empty printer list.
- **Tray/bin exact-match first** (A-L7); substring match remains as a
  fallback.
- **`printer_status` doc corrected** (A-L9): spooler-reported status,
  not a physical sensor read.
- Not changed (documented in the audit): `DeviceCapabilitiesW` −1
  capability defaults (M6) and the plain-`String` error type (M7) —
  both need model/FFI-wide changes that belong with a Windows build
  available; `classify_form`'s 72→80 mm canonicalization (M5/D-11) is
  preserved because downstream receipt logic consumes the canonical
  width today.

## App shell (Tauri)

App-crate edits are deliberately pattern-identical to code already
present in the same files; their compile gate is Windows CI (see the
verification report). JS-string edits were verified with the jsdom
suites.

- **Queue-admin local-only enforcement** (B-2). `retry_job_cmd`,
  `cancel_job_cmd`, `clear_finished_cmd`, `export_diagnostics` now
  call `check_local_only` in addition to `check_origin` (the exact
  call the nav/settings commands already make). No behavior change for
  the local Control Center; the remote origin was and remains denied
  (now by two independent layers).
- **Async submission commands** (B-3). `print_pdf` and `print_image`
  are `async fn`, matching the read commands; bodies unchanged.
- **Sparse byte-map rejection** (B-4) in payload deserialization.
- **Permission overlap resolved** (B-8). `open_odoo_origin` removed
  from `allow-queue-admin` (still granted to the main window via
  `allow-toolbar`); remote reach identical (13 commands).
- **Gate extended** (B-8). `tools/check_command_surface.py` models
  `toolbar.json` with a group allowlist, scans the shell's JS surfaces
  for `invoke()` names, and dropped a dead helper.
- **Bridge facade hardening** (B-6). Published via
  `Object.defineProperty` (non-writable, non-configurable) with a
  plain-assignment fallback; page scripts can no longer replace it.
- **Bridge argument cleanup** (B-12). Duplicate snake_case invoke
  arguments removed (5 call sites); Tauri's camelCase mapping is the
  single contract.
- **Watchdog SPA arming** (B-7). The boot watchdog re-checks on
  `pushState`/`popstate` transitions and self-guards with an armed
  flag, so SPA soft-navigation into the backend is covered.
- **Control Center (B-1, HIGH)**. All dynamically rendered action
  buttons (job retry/cancel, printer test/forms) now carry
  `data-action`/`data-*` attributes handled by one delegated listener;
  value interpolation into inline handlers is gone and the unsafe
  `escapeJsStr` helper was deleted.
- **Control Center (B-10)**. Cancel-job confirms first (new i18n key
  in both languages, parity kept); the job row shows the diagnostics
  ID under the job id; `refreshAll` has an in-flight guard.
- **Control Center origin validator (B-12)**. IPv6 literal check
  tightened (single `::`, no stray edge colons).

## Odoo addon (19.0.2.8.0)

- **Fetch fallback** (F1). Report fetches (text + PDF branches) catch
  network failures and return to Odoo's standard download flow.
- **Display-name device posts** (F4). Every job-status post now sends
  the printer's display name as `device` (handler create/watch/error
  posts, POS create/watch posts). Routing still uses the pattern; the
  registry, picker, and reprint prefill now key on the real name.
- **Race-safe job creation** (F5). `Job.create` runs in a savepoint;
  on the unique-key `IntegrityError` the controller re-reads the row
  and continues through the existing update path (state/device/
  attempts still applied) instead of returning a 500.
- **Read-only registry ACL** (F6). Manager device rule is now
  `1,0,0,0`; new test `test_device_registry_is_read_only_for_everyone`.
- **Watcher policy** (F7). Report and POS watchers stop after 5
  minutes without state progress (30-minute absolute cap), and post a
  visible "outcome unconfirmed" notification instead of going silent;
  the server record keeps converging for long-queued jobs.
- **Honest outcome wording** (B-11). Success reads "Sent … to …"
  (submission-stage truth), failure no longer claims "Nothing was
  printed" and tells the user to check the printer before reprinting.
- **In-flight guard fingerprint** (F8). The duplicate-submission guard
  key now includes a fingerprint of `action.data` and record ids, so
  two different wizard submissions of the same report cannot collide.
- **Prefill priority** (F9). In `resolveBridgeTarget`, a named
  (reprint) printer is no longer overridden by class preference.
- **POS key window anchored** (F12). Reuse no longer refreshes the
  15 s idempotency window timestamp.
- **Translatable JS + rebuilt Arabic catalog** (F2/F3). User-facing
  strings in the handler, POS, and systray go through `_t`
  (`@web/core/l10n/translation`); `i18n/ar.po` dropped 21 stale
  entries (deleted profile/rule features) and gained 51 current
  entries with Arabic translations — 110 entries, 0 untranslated.
  Static OWL template labels remain untranslated (documented).
- **Text corrections** (F10/F13). POS header comment now describes the
  real fallback; the groups XML comment no longer mentions the deleted
  rules; both reprint confirm dialogs describe the prefill behavior.

## Versions

- App 1.4.1 → **1.4.2** (tauri.conf.json, package.json, package-lock,
  Cargo.toml, app Cargo.lock — `check_versions.py` green).
- Addon 19.0.2.7.0 → **19.0.2.8.0** (manifest; fresh-install and
  upgrade suites green).

## Hotfix 1.4.3 / addon 19.0.2.9.0 (2026-11-04, field report)

- **White page after login (Arabic) — root cause + fix.** The
  mission-rebuilt `i18n/ar.po` had 51 entries missing the
  `#. module:` comment that Odoo's PO reader unconditionally matches;
  the reader raised, `/web/webclient/translations` returned 500 for
  ar_001, and the web client's localization service rejected during
  boot (no IndexedDB cache on a fresh registry) — login worked, the
  backend never mounted. Reproduced locally end-to-end (500 with an
  authenticated ar_001 session, identical 265-byte body to the
  production server), fixed by completing every entry's comment, and
  verified: endpoint 200 with all 24 web messages served in Arabic.
  The same pass added the missing `#. odoo-javascript` /
  `#. odoo-python` markers and `code:` occurrences — without them the
  catalog parsed but served zero code translations. A regression test
  (`TestTranslationFiles`) now loads the catalog in the addon suite.
- **Control Center.** Refresh gives feedback (spinner + disabled
  state + completion toast); native `window.confirm` replaced by a
  styled in-app modal everywhere; topbar/nav modernized (sticky
  glass, pill tabs); Paper Forms toggles per printer without
  stretching sibling cards.
- **Diagnostics.** The exported bundle reported `app_version: 0.1.0`
  (the print-core crate version); it now stamps the app version.
- **Brand.** Arabic product name changed to "الطباعة من Yaseir"
  (owner request) in the Control Center, toolbar, and Arabic strings.

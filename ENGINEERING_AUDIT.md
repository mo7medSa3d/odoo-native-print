# Engineering Audit — Yaseir Print / Odoo Native Print

**Cycle:** full-repository audit, repair, integration, printer-intelligence,
reliability and performance mission, 2026-11-04.
**Resulting versions:** desktop app **1.4.12** (from 1.4.11), Odoo addon
**19.0.2.15.0** (from 19.0.2.14.0). Tauri `>=2.11.1, <2.13`. Odoo 19 only.
**Scope note:** the out-of-scope areas from the mission (§0 — commercial
licensing/activation, code signing, anti-piracy, HTTP/HTTPS origin policy,
production server deployment) were **not** touched. No file in
`app/src-tauri/src/config.rs`, `build.rs`, permissions or capabilities
was modified in this cycle; the origin policy (ADR-0002) is unchanged.

**Evidence rule used throughout:** a claim is stated only if it was
verified against the code in this tree or against a test actually run
in this cycle. Earlier bundled reports described app 1.4.2 / addon
19.0.2.8.0 and are superseded by this document. Where verification was
impossible in this environment (Windows/MSVC, Odoo+PostgreSQL, physical
printers), the item is marked BLOCKED / NOT RUN, never passed.

## 1. Architecture overview

One product, four layers, one durable queue:

```
Odoo 19 page (report handler / POS) ── window.__NATIVE_PRINT__ bridge
  ── Tauri IPC (24 commands, 11 remote-reachable) ── Rust commands
  ── SQLite/WAL queue (durable, one transaction per job+payload+options)
  ── one worker per printer ── rendering (PDFium / PNG) ── transport
  (Win32 GDI spooler / Win32 RAW spooler / TCP 9100 / IPP)
  ── printer. Job outcomes return through the JS watchers to
  /native_print/job_status; Odoo keeps history only.
```

- The desktop worker owns physical execution. Odoo never executes a
  print and never stores printer truth: `native.print.device` is a
  history-only, auto-synced name registry used for log pickers and
  reprint prefill. The manifest wording that printers "are never
  stored here" refers to device truth, not to this registry; the
  model docstring is the authoritative description.
- A physical print is not an exactly-once transaction. Ambiguous
  outcomes (data may have reached the printer, acknowledgement
  missing) are recorded as ambiguous and are never auto-reprinted.
- Odoo works normally with no bridge: report actions fall back to
  the standard download; POS falls back to the browser print path.
- No local HTTP daemon, cloud relay, or new background agent was
  introduced.

Component map (verified against the code, not assumed from names):

| Component | Responsibility | Calls / called by |
|---|---|---|
| `crates/print-core` | Queue, state machine, paper ladder, routing, security, diagnostics, profiles, worker, health model | All crates; app dispatch |
| `crates/print-render` | PDFium loading/rendering, PDF preflight, PNG decode, bitmap, calibration, 203-dpi head profiles | App dispatch only |
| `crates/print-raw` | ESC/POS, ZPL, TSPL, EPL builders; TCP 9100; DLE EOT / TSPL / ZPL status parsers (library) | App dispatch, print-tools |
| `crates/print-ipp` | IPP 1.1 client, attribute parsing, blocking interpretation | App dispatch |
| `crates/print-win` | Win32 spooler/GDI FFI, discovery, capabilities, health reads | App dispatch; `npwin_probe` |
| `crates/print-tools` | Bench / chaos / calibration harness | Standalone binary |
| `app/src-tauri` | Tauri shell, 24 commands, bridge injection, worker loop, dispatch | Odoo pages via bridge |
| `odoo/native_print` | Job/device models, status controller, report + POS interception, systray | Desktop bridge; Odoo web client |

## 2. Components inspected

Every source file in the Rust workspace, the Tauri shell, the Odoo
addon (Python, JS, XML, ACLs, migrations, i18n), the Control Center
page, the tools and gates, and all four GitHub workflows was read.
Dependency versions were taken from the manifests and lockfiles, not
from documentation: workspace `Cargo.toml` (resolver 2, edition 2021,
MSRV 1.82; serde 1, serde_json 1, thiserror 2, rusqlite 0.32 bundled),
standalone `crates/print-win` and `app/src-tauri` workspaces, Tauri
`>=2.11.1, <2.13`, `Cargo.lock` files preserved.

## 3. Critical findings and fixes

### C1. One offline printer stalled every other printer (queue/worker)

**Root cause.** `Worker::run_once` slept the single worker thread for
the retry backoff (base 500 ms, max 30 s), and the worker loop in
`app/src-tauri` iterates printers serially. One unreachable printer
therefore blocked all healthy printers for up to 30 s per retry.

**Fix.** Backoff is now durable queue state, never a thread sleep:
schema migration adds `jobs.available_at` (user_version 2) with index
`idx_jobs_claim_available`; `claim_next` only claims jobs whose
`available_at` has passed; `Queue::requeue_with_delay` performs the
`printing → failed → queued` transition plus the delay in one
transaction; `retry_job` clears the delay (including for an already
queued backoff-waiting job, which previously errored on a
`queued → queued` transition); `recover_on_startup` clears delays
only for recovered in-flight rows. The `Worker` generic sleeper
parameter was removed; the app worker loop and the chaos harness
were updated to the new constructor. A worker test proves a job in
backoff is unclaimable while the worker thread stays free, and that
a manual retry clears the delay immediately.

### C2. Odoo froze a job on its first failure forever (sync/state machine)

**Root cause.** `controllers/main.py` treated `failed` and
`dead_letter` as terminal (`TERMINAL_STATES`, no exits in
`ALLOWED_TRANSITIONS`), but the desktop state machine allows
`failed → queued` (worker auto-retry) and `dead_letter → queued`
(operator retry). The JS watchers also stopped polling at the first
failure. A job that failed once and then printed stayed `failed` in
Odoo forever, and its successful outcome post was discarded as
`terminal_state_is_final`.

**Fix.** `TERMINAL_STATES` is now exactly `{done, canceled}`.
`failed`/`dead_letter` accept re-entry at `queued` (plus forward
skips for a lost `queued` post) in the controller. Both JS watchers
keep polling past retryable outcomes within their existing
5-minute-no-progress / 30-minute ladders: the report watcher
notifies once per (state, attempts) and continues; the POS watcher
hands the order back immediately on first failure (cashier UX
unchanged) but continues a background poll with once-per-outcome
posting so the retry's outcome still reaches the server log, and
never touches the order again after release. A new controller test
covers `failed → queued → done` and `dead_letter → queued → done`
plus continued finality of `done`. The Odoo suite itself could not
run here (no Odoo/PostgreSQL); the new test is Python-compiled only
and must run under `odoo.yml` CI.

### C3. Printer health collapsed simultaneous conditions and could show stale Ready

**Root cause.** A single priority-ordered `PrinterStatus` enum lost
simultaneous conditions (door open + paper out + toner low became
one value), had no observation timestamp, TTL, or source, and the
pre-print live-status read was fail-open (`if let Ok(...)`).

**Fix.** New `crates/print-core/src/health.rs`: `PrinterHealth`
with a sorted, deduplicated condition set, `HealthSource`
(spooler/IPP/ESC-POS/TSPL/ZPL/configured/unknown), `observed_at`,
30 s TTL, and raw evidence. `blocks_printing()` distinguishes
blockers from warnings (paper-low/toner-low do not block).
`from_spooler_flags`, `from_ipp`, `from_tspl_status`, and
`from_escpos` decoders preserve every reported condition.
`primary_status()` derives the single status from the same
observation, so badge and conditions cannot disagree. TTL 0
(`unknown`) is always stale. `print-win` exposes `printer_health()`
(a failed read is an error, never a fabricated Ready); the app
pre-print gate blocks on ANY blocking condition and keeps the
discovery-time status as the gate when the live read fails (a
failed read is never presented as Ready). The Control Center cards
show every condition, the source, and a STALE marker past TTL, and
the re-render signature includes conditions/staleness/stable id
(but not the raw timestamp, which would flicker the grid).

### C4. Capabilities were fabricated from driver errors

**Root cause.** `printer_capabilities` clamped `DeviceCapabilitiesW`
returning −1 — which the API does not distinguish from
"unsupported" — into concrete defaults (`duplex=false`,
`color=false`, `max_copies=1`) with no provenance. Consumers could
not tell a reported value from a fallback.

**Fix.** `CapabilityReport`/`CapabilityProvenance` per field
(`DriverReported`/`Inferred`/`Configured`/`Unknown`) plus
`PrinterCapabilities::from_driver_report`. Unknown queries keep
safe fallback values but are flagged `Unknown`. Legacy serialized
payloads without the field deserialize to all-`Unknown`. The
Control Center capability chips only ever appear for `true`
values, which under the new constructor imply a reported value.

### C5. No stable printer identity

**Root cause.** `PrinterInfo` had only name/driver/port. Display
names are not identities: renames, reinstalls, IP changes, and
multiple queues per device were unhandled.

**Fix.** `stable_printer_id` = FNV-1a hash of normalized driver +
uppercase port, for endpoint ports only (USB/COM/LPT/IP_/WSD/IPP/
IPv4); virtual/file ports yield no identity and never merge.
`same_physical_endpoint` compares identities. Devices are never
merged on name/model match alone; unresolvable identity stays
separate and is reported as ambiguous. **Limitation (honest):**
the level-2 spooler record exposes no device instance ID, USB
serial, or hardware id, so the identity does not survive a driver
reinstall or USB renumbering. The Control Center shows one card
per queue and does not auto-merge.

### C6. Rendering hardcoded 203 dpi; label gaps hardcoded

**Root cause.** `pdf_to_raw_stream`/`png_to_raw_stream` and the
label-box conversion rendered at a literal 203 dpi regardless of
the driver-reported resolutions, so a 300-dpi label/thermal device
received a mis-scaled raster. TSPL gap was a literal `3` and the
EPL gap a literal `24` at every call site; `JobOptions` had no gap
field, so no layer could carry a configured gap.

**Fix.** `preferred_raw_render_dpi` (203 if reported, else smallest
200–300 dpi, else smallest reported, else 203) is threaded through
one capabilities read: label fit box, PDF render DPI, TSPL mm
conversion (via the page's own DPI), and ESC/POS head-dot scaling
in `receipt_width_dots` (203-dpi head profiles scaled by the
reported resolution). `JobOptions.label_gap_mm` (serde default,
backward-compatible with stored rows) flows from the Tauri request
structs through the queue JSON to `TsplLabel::new` (`GAP`) and a
new `epl_label_copies_with_gap` (`Q` gap in dots at render DPI;
24 dots = 3 mm at 203 dpi remains the default). The raw-render
parameters were bundled into a `RawRender` struct after Clippy
rejected the 9-argument functions. The 203-dpi head-profile tables
in `print-render` are retained and documented as head profiles,
not universal truths; unknown widths are refused, never assumed.
PNG label sizing uses the render DPI with a documented caveat
(PNGs carry no intrinsic DPI). The 72 mm form-width
canonicalization in `classify_form` is unchanged (downstream
receipt logic depends on the canonical value) and is documented
as a known quirk.

### C7. IPP printer URIs were malformed for IPv6

**Root cause.** The dispatch built
`format!("ipp://{host}:{port}{path}")` from a host already
unbracketed by `parse_host_port`, producing an invalid URI for
IPv6 printers, while `IppClient::new` bracketed correctly for the
connection itself.

**Fix.** `NativePrintBackend::ipp_uri` re-brackets hosts containing
`:`, with round-trip unit tests for `[::1]`/`[2001:db8::1]`,
IPv4, and hostnames.

### C8. Protocol status parsers did not exist (library level)

**Root cause.** Only DLE EOT n=2/n=4 existed as unwired library
functions; no TSPL/ZPL status parsing existed at all; spooler
status was the only live status.

**Fix (library only, deliberately not wired into print paths).**
`query_printer_status` (n=1) and `query_error_status` (n=3) with
`decode_dle_eot_printer`/`decode_dle_eot_error` (n=2 was corrected
in documentation: it carries no cover-open bit); TSPL `ESC !?`
one-byte decoder tested against the Zebra guide hex table;
ZPL `~HS`/SGD `device.host_status` parser tested for both framings
and for short/empty responses (no response in MEDIA OUT / RIBBON
OUT / HEAD OPEN / over-temperature is fault evidence, never
readiness). Wiring a probe into a print stream would risk
corrupting print data and needs a separate bounded bidirectional
probe plus hardware validation; it was not done. GS r and ASB
remain not implemented. See
`PRINTER_COMPATIBILITY_MATRIX.md` for the per-protocol tables and
sources. Protocol research used published documentation excerpts
(secondary sources); official manuals were not fetched directly
in this environment, and no hardware validation was run.

## 4. High-priority findings (verified, kept, or documented)

- **State machine.** One authoritative transition contract now
  exists in Rust (`state.rs`) and is mirrored in the Odoo
  controller; duplicate posts are harmless; attempts are monotonic
  on both sides; delayed posts cannot overwrite a newer outcome
  (the controller rejects backward moves and exits from
  `done`/`canceled`; the queue uses guarded
  `UPDATE ... WHERE state=<old>`).
- **Queue durability.** WAL + `synchronous=FULL` + 30 s busy
  timeout; job+payload+options in one IMMEDIATE transaction;
  duplicate idempotency keys return the original untouched and a
  conflicting payload records a diagnostic event; limits 500 /
  printer and 2000 global are unchanged and exercised by the
  harness; one worker per printer (claim ordering priority, then
  seq); `reconcile_stale` covers rendering/spooling;
  `recover_on_startup` requeues rendering/spooling and fails
  printing jobs as `RECOVERY_AMBIGUOUS` unless auto-reprint is
  set. Migrations are idempotent and non-destructive (v2 adds a
  column + index).
- **Win32 FFI.** RAW write loop until all bytes accepted,
  `AbortPrinter` on failure, error captured before cleanup, u32
  chunk guard; GDI uses per-job `DEVMODEW` validation and never
  mutates global defaults. FFI bodies are Windows-gated and were
  **not** compiled in this environment — Windows CI is the gate.
- **Transports.** TCP 9100 errors are classified ambiguous; no
  auto-retry after partial delivery. IPP negotiates PDF only
  (no raster encoder exists; raster-only printers get
  `NoUsableFormat`), honors `is_blocked()`, caps responses at
  1 MiB, and validates options against reported capabilities
  instead of dropping them.
- **Media matching.** Deterministic ladder (exact ±1 mm →
  roll-width fit → policy-gated custom size → nearest larger);
  never returns a smaller form, never silently crops/scales;
  `PAPER_MISMATCH` is actionable. Windows paper sizes are tenths
  of a millimetre with array-count validation.
- **Rendering.** PDF preflight caps page counts before allocation;
  PNG decoding enforces 20 MiB / 50 Mpx / 20,000-side limits;
  banded ESC/POS output (512 rows); Arabic goes through
  rasterization, never raw UTF-8 to an assumed code page.
- **Command surface / bridge.** The gate passes: 24 commands,
  remote reaches 11, admin stays local. The bridge facade is
  non-writable/non-configurable, top-frame only, normalizes
  `Uint8Array`/`ArrayBuffer`/Base64/sparse-map payloads, and
  never reports an accepted job as printed.
- **SNMP.** Not implemented, deliberately: IPP + driver status
  cover the supported scope; SNMP would only add supply levels on
  non-IPP office printers and needs its own credential design
  (no default community strings). Documented in the matrix.

## 5. Medium-priority findings

- `job_detail` does not expose `diag_id`/timestamps/transport in
  the Tauri command response; the Control Center shows `diag_id`
  from the queue bundle path instead. Documented gap; not changed
  (the command surface gate pins the response shape).
- Option preservation is not complete end-to-end: orientation,
  page range, resolution, density, and cut/feed specifics beyond
  `cut: true` are not carried from Odoo/JS to the backends.
  Carried options (printer, mode, paper, copies, duplex, color,
  tray, scale policy, roll width, label gap) are preserved; the
  rest are documented as a known gap rather than claimed as
  supported.
- POS auto-retry vs deliberate reprint: a transiently failed POS
  job may be auto-retried by the worker after the watcher has
  handed the order back, so a manual reprint can produce a second
  physical receipt. Pre-existing behavior, unchanged by this
  cycle; the retry outcome is now at least visible in the Odoo
  log. Recorded as a known limitation.
- No criterion benchmarks ship; performance evidence is the
  `print-tools` harness (see verification report). Discovery uses
  a 5 s snapshot cache with single-flight refresh; status polling
  is bounded by TTL + backoff.
- The jsdom watchdog suite still labels its scenarios "1.4.7";
  cosmetic only, left untouched.

## 6. Files changed

New: `crates/print-core/src/health.rs`.
Modified: `crates/print-core/src/{lib,printer,queue,worker,paper,routing}.rs`,
`crates/print-raw/src/{lib,tcp9100,tspl,zpl,epl}.rs`,
`crates/print-win/src/{lib,discovery,win}.rs`,
`crates/tools/src/main.rs`, `app/src-tauri/src/commands.rs`,
`app/dist/index.html`,
`odoo/native_print/{controllers/main.py,tests/test_controllers.py,
static/src/js/native_print_handler.js,static/src/js/pos_native_print.js}`,
version metadata (app 1.4.12, addon 19.0.2.15.0),
`PRINTER_COMPATIBILITY_MATRIX.md`, `README.md`, `PROGRESS.md`, and
the three reports rewritten for this cycle. Formatting-only
normalization from `cargo fmt` is included in the print-win files.
Untouched by design: licensing/origin/security policy code,
permissions, capabilities, workflows, migrations, translations.

## 7. Remaining limitations and risks requiring real hardware / CI

1. Windows/MSVC compilation, MSI/NSIS builds, PDFium bundling on
   Windows, and all Win32 FFI behavior: BLOCKED here, gated by
   GitHub CI (`windows`, `windows-installers-unsigned`).
2. Odoo 19 + PostgreSQL install/upgrade/migration and the new
   retry-sync controller test: BLOCKED here, gated by `odoo.yml`.
3. All physical-printer validation (thermal USB/network, Zebra,
   TSC, laser, inkjet, IPP hardware, virtual-printer output):
   NOT RUN. `docs/hardware-test-matrix.md` cells remain unchecked.
4. Protocol parsers are table-tested against documentation
   excerpts, not against devices; model-specific differences
   (especially TSPL cover bit, ZPL SGD no-response states, ESC/POS
   clone behavior) need device confirmation.
5. Stable printer identity does not survive driver reinstall or
   USB renumbering (no stronger identifier in the spooler record).
6. Dependency vulnerability audit: see verification report.

## 8. Deferred out-of-scope matters

Licensing/activation, device limits, code signing, anti-piracy,
origin-policy redesign, and production deployment were preserved
as-is per the mission boundaries and are not addressed here.

# Engineering Audit — Yaseir Print (Odoo Native Print Client)

Date: 2026-11-04. Scope: the complete supplied project ZIP (app 1.4.1 /
addon 19.0.2.7.0 as received), audited in four parallel tracks and then
repaired. This report describes what was inspected, what was found, what
was fixed, and what honestly remains. Companion documents:
`VERIFICATION_REPORT.md` (evidence), `CHANGELOG_ENGINEERING.md` (every
change and its root cause), `PRINTER_COMPATIBILITY_MATRIX.md` (protocol
and sensor truth table).

## Architecture overview (as found, verified against the code)

- **Desktop app** — Tauri 2.12.5 shell (`app/src-tauri`) embedding the
  customer's Odoo in a WebView2 webview. A bridge facade
  (`window.__NATIVE_PRINT__`) is injected into the Odoo page; 23 Tauri
  commands connect the page, the local Control Center
  (`app/dist/index.html`), and the Rust core. A single background worker
  drains a durable SQLite queue (WAL, synchronous=FULL) and executes
  jobs through the backends. Local-only administration is enforced by
  capability ACLs and (after this mission) by Rust-side origin checks.
- **Rust workspace** — `print-core` (queue, state machine, paper ladder,
  routing, security, diagnostics), `print-render` (PDFium rendering,
  raster conversion), `print-raw` (ESC/POS, ZPL, TSPL, EPL builders, TCP
  9100 transport, DLE EOT status), `print-ipp` (IPP client),
  `print-win` (standalone crate: Win32 spooler/GDI FFI + platform-free
  discovery logic), `print-tools` (probe/chaos harness binary).
- **Odoo addon** — `odoo/native_print` (19.0.2.8.0 after this mission):
  report/POS interception in the page, a read-only job log with reprint,
  an auto-synced printer-name registry used for history pickers only
  (printing itself resolves live on the PC — by the owner's design),
  a systray widget, and a status-sync controller enforcing the same
  state machine as the desktop queue.
- **Print paths** — Windows GDI, Windows RAW spooler, direct TCP 9100,
  IPP, ESC/POS raster/text, ZPL/TSPL/EPL label programs, PDF via PDFium,
  PNG receipts. Fallback to Odoo's standard download whenever native
  printing cannot safely complete.

## Critical findings

### C-1 — Stored script injection into the Control Center (fixed)
Dynamically rendered job/printer buttons interpolated an attacker-
influenceable value (a job's idempotency key, accepted from the remote
Odoo origin as any printable ASCII) into an inline `onclick` handler
using a hand-rolled double-context escaper. HTML attribute decoding
defeated the escaping, so a crafted key could break out of the handler
string and execute script in the local admin page — a page that can
call every command, including origin changes and diagnostics export.
Fixed by carrying values in `data-*` attributes behind one delegated
click listener; the unsafe escaper was deleted. (Track B-1.)

## High findings

### H-1 — Partial RAW spooler writes were committed as complete jobs (fixed)
The spooler RAW path issued a single `WritePrinter` call and then
closed the document unconditionally before checking the result: a
partial write became a truncated physical page **and** an error return,
so a retry printed a duplicate. Fixed with a write loop over the
remaining bytes, `AbortPrinter` on failure (the FFI declaration did not
exist), error capture before cleanup calls could clobber it, and a
u32-length guard. Windows-CI compilation is the verification gate.
(Tracks A-H1 / D-3.)

### H-2 — IPP format negotiation promised formats the product cannot produce (fixed)
The IPP client preferred PDF, then PWG Raster, URF, JPEG — but no
raster encoder exists anywhere in the product, so a non-PDF printer
would have received raw PDF bytes labeled as a raster format. The
preference list is now PDF-only: such printers are rejected loudly with
the existing `NoUsableFormat` error until a real encoder exists.
(Track D-1.)

### H-3 — Windows status decoder ignored documented intervention flags (fixed)
`PRINTER_STATUS_USER_INTERVENTION`, `PAPER_PROBLEM`, `MANUAL_FEED`,
`OUTPUT_BIN_FULL`, `NOT_AVAILABLE`, and `BUSY` were not decoded; a
printer reporting only user-intervention decoded to `Unknown`, which
every consumer treats as printable. All are now decoded (intervention
flags map to a new `UserIntervention` status that blocks printing;
paper-problem maps to `Error`; busy maps to the printing class).
(Tracks A-L10 / D-2.)

## Medium findings (fixed unless noted)

- **Queue `set_state` was a non-atomic read-check-update** — two
  connections could interleave transitions the machine forbids. The
  write is now guarded on the previously read state; the loser gets a
  transition error. (A-M3.)
- **Manual retry reset `attempts` to 0**, breaking the
  attempts-never-decrease invariant. Retry no longer resets it. (A-M1.)
- **Stale-job reconciliation could fail a legitimately printing job**
  (no heartbeat exists; a >15-minute physical print looked wedged).
  Reconciliation now covers `rendering`/`spooling` only; `printing` is
  startup recovery's domain, where the ambiguous-outcome policy lives.
  (A-M2.)
- **PDFium lock poisoning** — a panic under the process-wide PDFium
  lock (one `.expect` sat inside the locked region) permanently killed
  the PDF path for the process. Locks now recover from poisoning; the
  in-lock expect became a per-job error. (A-M10.)
- **Queue-admin commands lacked the Rust-side local-only check** and
  relied on the capability ACL alone. `retry_job_cmd`, `cancel_job_cmd`,
  `clear_finished_cmd`, `export_diagnostics` now also call the same
  local-only check the navigation/settings commands already used.
  (B-2.)
- **Heavy submission commands ran on the UI thread** (`print_pdf`,
  `print_image` were synchronous Tauri commands and parse up to 50 MB
  there). Both are now `async`, like the read commands already were.
  (B-3.)
- **Sparse byte-map payloads silently compressed** — a `{0:…, 2:…}`
  IPC payload deserialized into a shorter, shifted byte string. Gaps
  are now rejected at deserialization. (B-4.)
- **The print worker re-ran full Windows discovery for every job**,
  serially (the UI had a cache; the execution path did not). print-win
  discovery now goes through a process-level single-flight snapshot
  cache (5 s TTL, unit-tested), so a 50-receipt burst shares
  enumerations. (B-5.)
- **Report-download fallback broke on network failure** — the report
  fetch in the Odoo handler was not guarded, so a rejected fetch escaped
  the handler as an exception instead of returning to Odoo's standard
  download. Both branches now catch and fall back. (C-F1.)
- **Job log registered printers by routing pattern, not name** — for
  network raw printers the registry (and therefore the reprint prefill
  and the picker) carried `tcp://<ip>:9100` instead of the printer's
  display name, so reprint prefill silently never matched. All job
  posts now carry the display name; the pattern remains a routing
  input only. (C-F4.)
- **Job creation race** — concurrent first posts for one key+company
  could 500 on the unique constraint (device registration already had
  the savepoint treatment; job creation did not). Mirrored the pattern;
  the loser re-reads the row and continues through the update path.
  (C-F5.)
- **Printer registry was manager-editable**, contradicting its
  auto-synced, history-only design (a rename/delete corrupts job
  history). Manager ACL is now read-only like everyone else's; a
  regression test asserts all device rules are read-only. (C-F6.)
- **Arabic translations were stale** — 21 entries referenced the
  printer-profile/rule features deleted in 19.0.2.0.0, and most current
  strings (job log UI, all JS notifications) had no entries at all;
  addon JS used no `_t`, so its strings were untranslatable. The `.po`
  was rebuilt (stale dropped, 51 current strings translated) and the
  user-facing JS strings in the handler, POS, and systray now go
  through `_t`. (C-F2/F3.)
- **IPP printer status was collected but never interpreted** —
  `printer-state-reasons` were stored and ignored, and
  `printer-is-accepting-jobs` was never requested. The IPP layer now
  requests and parses accepting-jobs and exposes `is_blocked()` /
  `blocking_reasons()` (media-empty, media-jam, door-open, cover-open,
  marker-supply-empty, offline, input-tray-missing, output-full;
  warnings such as toner-low do not block). One consumer call-site swap
  in the app crate (`is_stopped` → `is_blocked`) is documented in the
  changelog as the remaining step, gated on the Windows build.
  (D-8, partial.)
- **ESC/POS paper status (DLE EOT 4) did not exist** and the n=2
  decoder's comment misattributed the paper-end bits to n=2. The
  comment is corrected and a DLE EOT 4 query + decoder with a bit-table
  test was added. Wiring a pre-flight status query into the TCP print
  path remains a documented follow-up (it changes print-path behavior
  and belongs with hardware validation). (D-4/D-5, partial.)
- **Command-surface gate blind spots** — the gate did not model
  `capabilities/toolbar.json` and never scanned JS call sites. It does
  both now, and immediately caught a real overlap: `open_odoo_origin`
  sat in both the queue-admin and toolbar permission groups. It was
  removed from queue-admin (the main window keeps it via the toolbar
  group; remote reach is unchanged). (B-8.)

## Lower-severity findings fixed

Queue: duplicate-conflict detection compared only payload size (now
compares full payload bytes); unknown state strings in the DB were
silently coerced to `failed` (now a database error — corruption must be
visible); a `PRAGMA user_version` schema marker was added for future
migrations. Printer classification treated local serial/parallel/file
ports (`COM1:`, `FILE:`) as network printers because they contain a
colon; local port prefixes are now matched first, and IPv4 validation
checks octet ranges. Printer-quirk profile lookup was nondeterministic
when several driver keys matched (HashMap order); the longest key now
wins deterministically. TSPL `TEXT` content was sanitized only for
quotes — embedded CR/LF could inject arbitrary TSPL commands; control
bytes are now stripped (the ESC/POS builder already did this). TCP
status reads decoded the first byte received, including possible ASB
noise; DLE EOT frames are now validated against their fixed bits with
bounded retries. IPP format matching is base-type based (parameters
and case no longer cause false rejections). Tray/bin resolution now
prefers an exact name match ("Tray 1" could resolve to "Tray 10").
A failed first `EnumPrinters` probe was indistinguishable from "no
printers"; it now propagates as an error. Bridge: the facade is now
published non-writable/non-configurable so page scripts cannot replace
it; duplicate snake_case invoke arguments were removed (Tauri maps
camelCase); the boot watchdog now also arms on SPA soft-navigation
into the backend. Control Center: cancel-job asks for confirmation
like the other destructive actions; the diagnostics ID is shown with
each job; the 5 s refresh can no longer overlap itself; the IPv6
literal check rejects malformed forms. Odoo: watchers now give up
only after 5 minutes without state progress (30-minute cap) instead of
a fixed 5 minutes, and report an unconfirmed outcome rather than going
silent; the in-flight duplicate guard now fingerprints wizard data and
record ids (two different submissions of one report could collide);
the POS idempotency window is anchored at first submission instead of
sliding on every reuse; a stale POS header comment and two stale UI
texts were corrected; `resolveBridgeTarget` no longer lets class
preference override an explicit reprint-prefill printer name.

## Known limitations (documented, not fixed — with reasons)

- **Status model shape** — printer status remains a single primary
  status (priority-ordered) plus a toner-low flag; there is no
  per-sensor multi-condition object and no observation timestamp on
  `PrinterInfo`. Extending the struct ripples into the app crate and
  the JS contract; it should land with the Windows build available.
  The pre-print gate in the app worker also still treats a failed
  live status read as non-blocking (fail-open); making it fail-closed
  is a one-line app-crate change that must be verified on Windows.
- **Raw rendering DPI** — raw modes render at 203 dpi; the driver's
  reported resolutions are collected but not yet used to pick the
  render DPI, so 300 dpi thermal/label devices receive a 203 dpi
  raster (physically mis-scaled). The fix lives in the app crate's
  dispatch and needs Windows verification.
- **TCP paper-width fallback** — when a direct-TCP job lacks synced
  roll metadata, one legacy fallback still maps unknown paper names
  to an 80 mm raster (the spooler path already refuses to guess).
  Changing the shared function's signature ripples into app call
  sites; documented here as the next cleanup.
- **Roll-form canonicalization** — discovery rewrites a ~72 mm form
  width to 80.0 mm before storing it. Downstream receipt logic depends
  on the canonical value today; splitting "reported" vs "canonical"
  width is a data-model change for the next cycle.
- **Label media descriptors** — TSPL gap is a job-independent 3 mm
  default in the app dispatch and the EPL builder's gap is a constant
  24 dots; black-mark (BLINE) media is not expressible yet. The TSPL
  builder already accepts a gap parameter; the EPL builder needs one,
  threaded from job options.
- **Capability fabrication on driver error** — when
  `DeviceCapabilitiesW` returns −1 (driver error vs unsupported is
  indistinguishable at that API), capabilities fall back to concrete
  defaults (`color=false`, `max_copies=1`). The capability model has
  no "unknown" representation for these fields yet.
- **GDI multi-subjob partial failure** — print-win errors are plain
  strings; a PDF split into subjobs that fails midway cannot express
  "earlier subjobs already printed" structurally, so the app layer
  cannot classify it as precisely as the TCP path. An error-enum
  refactor of print-win is the fix.
- **SNMP / Printer-MIB** — not implemented. IPP attributes and driver
  status cover the supported network scope; SNMP would add value only
  for supply levels on non-IPP office printers and needs its own
  credential-handling design (no default community strings).
- **TSPL/ZPL device status** — `ESC !?` and `~HS`/SGD status are not
  implemented; TSC/Zebra status in the app is the Windows spooler
  status only. ESC/POS `GS r` and ASB are likewise not implemented
  (DLE EOT 2 + 4 exist as library functions with tests).
- **Visual toolbar** — the child-webview toolbar page, capability, and
  navigation commands ship inert by the owner's earlier decision
  (every toolbar-block variant broke the Windows installer build in
  CI runs #68–#75; the block-free tree is the proven-green
  configuration). They are now modeled by the command-surface gate so
  they cannot drift silently. Restore only with a Windows compiler
  log in hand.
- **`__pycache__` in the source tree** — Python bytecode caches are
  excluded from the delivered ZIP (the build script's exclusion list
  was verified against the final archive listing).

## Out of scope (per the mission brief, untouched)

Licensing/activation, code signing, and the HTTP/HTTPS origin policy
(ADR-0002) were not modified. The origin policy code paths were read
for context only and verified unchanged.

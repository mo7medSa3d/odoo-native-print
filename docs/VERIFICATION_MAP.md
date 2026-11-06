# Verification Map — Yaseir Print (app 1.5.6 / addon 19.0.2.21.0)

Full-project verification of the shipped ZIP (sha256
fcbebdc614f66b03b072585c0aa7d1516b1ebbc1ab4d4a495323aee30cf84880),
run on the extracted tree, 2026-11-06. Each node lists its scope,
the checks run, and the evidence. Status: PASS / FIXED / OPEN.
This map is the working reference: later rounds re-check nodes
whose inputs changed.

## A. Artifact integrity
- A1 ZIP == GitHub main 209ff237 file-for-file.
  Evidence: recursive diff vs a fresh codeload snapshot.
  Result: identical except PROGRESS.md (local full ledger vs the
  GitHub prefix — by design), one Cargo.lock version line (F2),
  and known inert extras (F1). PASS w/ findings F1, F2.

## B. Build & gates (extracted tree) — ALL PASS
- B1 cargo test --workspace: 140/140 (71 worker/queue, 24 tcp,
  33 core, 8 + 4 misc).
- B2 cargo test (app crate): 10/10.
- B3 clippy workspace + print-win, -D warnings: clean.
- B4 per-file rustfmt (stable + 1.90.0) on app + print-win
  sources: clean.
- B5 tools/check_versions.py: app 1.5.6 everywhere; addon
  19.0.2.21.0 on its independent line.
- B6 tools/check_command_surface.py: 32 commands consistent
  across handler/manifest/permissions/capabilities; remote
  reaches 13; admin stays local.

## C. print-core — PASS (unit suites + token-parity suite)
- C1 Origin policy (ADR-0002): HTTPS any host; HTTP any IP
  literal; HTTP hostname rejected. Unit tests + parity across
  app config validator, Tauri capabilities, CC JS.
- C2 classify / detect_protocol: name tokens, OneNote / NUL: ->
  Virtual, protocol + class on every PrinterInfo; JS twin tables
  identical to Rust (node live-target suite).
- C3 Payload limits: 50 MB / 2000 pages / 99 copies / 1500 mm;
  idempotency-key validation.
- C4 document_mode_downgrade: document + raw mode + Virtual ->
  GDI; true raw payloads stay blocked to virtual printers.

## D. Queue & job lifecycle (desktop) — PASS (worker/queue suites, 71 tests)
- D1 Enqueue: job + payload + options in ONE SQLite transaction;
  duplicate key returns the original job untouched.
- D2 States: queued -> rendering -> spooling -> printing ->
  done | failed | canceled | dead_letter; retry envelope
  6 attempts with durable backoff (available_at, schema v2).
- D3 Stale-job reconciliation (900 s) cannot fire mid-print
  (single worker thread); 2 s worker wake.
- D4 Ambiguity rule (ADR-0004): write-phase failures never
  auto-retry; pre-write failures are retryable / rescuable.

## E. Worker dispatch matrix — PASS (worker suites + code audit)
- E1 (pdf, gdi) rendered via PDFium -> GDI; (pdf, raw-class
  modes) downgraded or spooled per class rules.
- E2 (image-png, escpos/zpl/tspl/epl): raw via queue or
  tcp://IP:9100 (connect-phase retries; write-phase ambiguous).
- E3 (image-png, gdi): canvas path — caps-configured media,
  default-media fallback on any pre-print failure, tall-strip
  pagination, virtual targets skip physical status/health gates.
- E4 (raw bytes, *): spooler RAW; pre-write refusal of an image
  or PDF job rescues into GDI in the same attempt; raw payload
  to virtual printer is hard-blocked.
- E5 Receipt width evidence ladder: configured paper -> job
  media width -> driver roll form -> narrowest receipt-range
  form (>30 mm disagreement rule) -> receipt source_width_mm;
  failure only with zero evidence.

## F. print-win (Windows FFI; compile-proven by CI) — PASS (CI 209ff237 + clippy)
- F1 Discovery: EnumPrinters + driver/port/status words;
  PRINTER_STATUS_NOT_AVAILABLE decodes as Unknown (printable).
- F2 Capabilities: forms (roll flags), trays, duplex, color,
  DPI; IPP tray media-source mapping, unmappable tray fails
  loud (IPP_OPTION_UNSUPPORTED).
- F3 print_raw_phased: pre-write vs write-phase error split.
- F4 Canvas GDI: DEVMODE media selection; null-DEVMODE default
  fallback; shared sheet pagination.

## G. print-render — PASS (render suites)
- G1 PDFium rendering at device DPI; page-size ladder
  (ADR-0003) fails loud on a wrong size for documents.
- G2 PNG decode limits: side <= 40k px, <= 50 Mpx;
  receipt raster cap 2048 dots wide.

## H. Bridge contract (webview <-> Rust) — PASS (surface gate + call-site scan)
- H1 Every BRIDGE_JS method maps to a registered command
  (surface gate, node B6) and every page call site uses only
  defined methods.
- H2 print_image / print_pdf argument contracts match the page
  payloads field-for-field (camelCase serde).
- H3 Remote-reachable commands = 13, all origin-checked;
  queue-admin + tab commands stay local-only.

## I. Page JS — reports (native_print_client.js) — PASS (node suites + code audit)
- I1 Interception scope: report prints only; Excel/CSV and
  other downloads untouched; plain browser keeps Odoo flow.
- I2 Target order: reprint-named -> selected (any class) ->
  class preference -> Windows default -> first printer.
- I3 No resolvable printer inside the app: notify BEFORE the
  download fallback (never a silent substitution).
- I4 Systray/CC live printer list with per-printer source meta.

## J. Page JS — POS (pos_native_print.js) — PASS (node suites + live controller proof)
- J1 Wrappers: PosStore.printReceipt + ReceiptScreen — inside
  the app, silent native or a reasoned failure; the browser
  print dialog is never invoked; plain browser untouched.
- J2 Capture: clone + inline styles/media; unfetchable media
  degrades to a transparent pixel (never an external URL in the
  SVG); scale 2 with scale-1 retry; canvas handles QR canvases.
- J3 Keys: per-cashier idempotency keys, 15 s dedupe window,
  rotation on terminal outcome and on decline.
- J4 Watcher: outcome posts converge (failed -> queued
  re-entry); order._printed follows the CONFIRMED outcome;
  cashier told on failure with printer + error code.
- J5 Declined attempts post a coded row (POS_NO_PRINTER /
  POS_RECEIPT_NOT_ON_SCREEN / POS_CAPTURE_FAILED /
  POS_ENQUEUE_FAILED) — proven live on Odoo 19 (row created).

## K. Odoo controllers — PASS (suite + live posts)
- K1 /native_print/job_status: group gate; key <= 128 chars;
  state machine transitions; TERMINAL_STATES = {done, canceled}
  are final; per-user scoping; race-safe creation.
- K2 /native_print/sync_device: upsert onto unique
  (name, company) device row; no duplicates under races.
- K3 Overview / diagnostics endpoints: company scoping.

## L. Odoo models & security — PASS (suite: ACL/reprint/registry tests)
- L1 Job log: read-only views + ACLs; creation only via the
  status controller; printer = device Many2one; source fields;
  print-settings snapshot on every job.
- L2 action_reprint: administrators always qualify; others via
  implied-groups closure (proven root cause, 1.5.0); POS jobs
  raise a loud error; prefilled printer + settings.
- L3 Retention cron present but INACTIVE (180 d, terminal only).

## M. Odoo suites & bundles — ALL PASS
- M1 Fresh install suite ON THE EXTRACTED TREE: native_print
  37 tests, 0 failed / 0 errors (the one ERROR line in the log
  is the idempotency race test's expected duplicate-key SQL).
- M2 Upgrade suite .20.0 -> .21.0: 0 failed, 0 errors of
  29 tests (run on the byte-identical intake tree).
- M3 Live bundles on the upgraded DB: POS bundle (4.27 MB)
  contains postPosDeclined + POS_NO_PRINTER +
  POS_ENQUEUE_FAILED + the reason strings + the transparent
  pixel stand-in; web bundle contains the current thermal
  tokens; SCSS scan: no min()/max()/clamp() mixed-unit
  constructs anywhere (the 2026-11-04 bundle-killer pattern).

## N. Shell UI (local pages) — PASS (jsdom smokes: tabbar today; CC/settings in their rounds)
- N1 Tab bar: tabs from tab_list; POS badge; + inside the strip
  after the last tab; nav group beside Control Center/Settings;
  drag-and-drop reorder via tab_move (jsdom smoke).
- N2 Control Center: full-width top bar; section tabs as a
  distinct second bar; per-class printer cards with capability
  chips; per-printer defaults panel scoped to its own card;
  lab kinds rebuilt per selected printer.
- N3 Settings page: About with live version + Yaseir credit;
  preferences persist.
- N4 Boot page: origin validation mirrors the core policy;
  portal-pattern branding (royal/cyan/orange palette).

## O. Security & privacy — PASS (scans + parity checks)
- O1 No secrets, tokens, or credentials in the tree (grep).
- O2 Origin allowlist parity in all four enforcement points.
- O3 Payload caps enforced at enqueue; file paths from the
  page never reach the filesystem unchecked.
- O4 Subframe origin trust = ADR-0005 (accepted, recorded).

## P. Docs & decisions consistency — PASS (ADR files match shipped behaviour)
- P1 ADRs 0002/0003/0004/0005/0006 match the code as shipped.
- P2 PROGRESS.md ledger covers every shipped version;
  MEMORY.md facts match the tree (spot-checked).

## Q. External comparison (open source) — researched 2026-11-06
- Q1 Ecosystem baseline: the dominant no-IoT-Box pattern is a
  local agent + server-stored printers (OCA pos_printing_qztray
  / base_report_to_printer_qz via QZ Tray; PrintNode syncs
  printers + paper sizes into the ERP; Frappe splits CUPS/QZ).
  This design (no agent process, no server printer state) is
  leaner than all of them.
- Q2 Verdicts: (1) live discovery at click time —
  DIVERGENT-BUT-REASONABLE (live reads are QZ-normal via
  qz.printers.details(); storing nothing is the owner's live
  printer model, cost = no central capability audit);
  (2) PNG/PDF payload split + class routing + raw->GDI
  downgrade + never-raw-to-virtual — CONFIRMED (mirrors QZ's
  pixel vs raw type system and its virtual-printer filtering);
  (3) ambiguous post-write never retried, pre-write retried —
  CONFIRMED (ESC/POS has no ACK channel; standard exactly-once
  doctrine; matches the documented Win32 RAW sequence);
  (4) width evidence ladder + sheet pagination — CONFIRMED
  (python-escpos profile widths + print-anyway fallback; the
  ladder works in mm, so the 180 vs 203 dpi caveat is covered);
  (5) idempotency keys + local queue + read-only server log —
  CONFIRMED (kiosk-printing reference pattern).
- Q3 Documented pitfalls, checked against this code:
  ESC @ job init — PRESENT (EscPos::new starts 1B 40);
  raster banding vs printer buffer overflow — PRESENT
  (MAX_RASTER_SLICE_HEIGHT = 512 dots, more conservative than
  python-escpos's 960-row fragments); PDF transparency
  flattening — PRESENT (bitmap pre-filled white 0xFFFFFFFF);
  roll-media GDI form trap — handled by canvas pagination +
  driver forms + default-media fallback (nodes E3/F4);
  idempotency key without content hash — bounded by the
  per-attempt key rotation at terminal outcomes (node J3).
  Field-guidance only (no code action): Star printers ship in
  Star Line Mode, not ESC/POS — switch emulation in the printer
  utility; post-PrintNightmare Windows may list a printer in
  EnumPrinters while RAW jobs need an admin-approved driver.

## R. Packaging
- R1 Final ZIP hygiene: no target/, no gen/, no
  permissions/autogenerated/, no __pycache__, no pdfium blobs.
- R2 CI for the exact final commit: Windows installer artifact
  present; Odoo workflow green.

## Findings
- F1 (packaging, minor) — FIXED in the final ZIP: the first
  1.5.6 ZIP shipped app/src-tauri/permissions/autogenerated/
  (build-generated). Excluded from the final ZIP. No runtime
  impact (Tauri regenerates it at build).
- F2 (consistency, minor) — FIXED: GitHub's
  app/src-tauri/Cargo.lock still said 1.5.5 (only the
  workspace-root lock was pushed). The updated file (1.5.6)
  is pushed with this map.
- No code defects were found by this verification. Every
  runtime behaviour change the research suggested checking
  (ESC @ init, raster banding, PDF transparency flattening,
  roll-media GDI handling) was already present in the code,
  with the code evidence recorded in node Q3.

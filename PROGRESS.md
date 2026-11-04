# PROGRESS

Last updated: 2026-11-03 (Africa/Cairo) — v1.3.0 hardening
Owner spec: Master Build Prompt — Odoo Native Print Client (Windows, then Android, then iOS)

## Status: v1.3.0 — full hardening pass (21-point plan, all four phases)

App 1.3.0 / addon 19.0.2.3.0. Detail: docs/engineering-hardening-2026-11-03.md.
Headlines: the Tauri command surface is now consistent across
lib.rs, build.rs, permissions and capabilities — guarded by
tools/check_command_surface.py in CI (v1.2.0's two new commands were
missing from build.rs and a permission group, which would have
denied them at runtime). Submission validates before parsing and
enqueues job+payload+options in ONE SQLite transaction (proven
under a two-connection race). Global queue cap (2000) joins the
per-printer cap (500). Wedged jobs are reconciled
(RECONCILED_STALE); every job has a diagnostics id and a structured
event log folded into the diagnostics bundle. The Odoo job_status
endpoint enforces a formal forward-only state machine. POS receipts
use attempt-based idempotency keys (double-fire dedupes, deliberate
reprint is a new attempt) and watch for ~5 minutes like reports;
report submissions have an in-flight double-click guard. HTTP
origins show a per-session unencrypted warning strip. Control
Center has a real CSP. POS receipt banding/downscale covered by
tests. Version metadata is gated (tools/check_versions.py) and
release signing is wired: configuring the WINDOWS_CERTIFICATE_*
secrets turns signed installers on with no code change — the
certificate purchase remains the only blocker there, together with
the printer-lab hardware matrix (docs/hardware-test-matrix.md).

Whole-system audit (2026-11-03, docs/system-audit-2026-11-03.md):
every cross-surface contract mechanically compared (bridge, ACL,
states, endpoints, manifest, DOM ids, i18n, origin policy) — all
consistent; zero unwrap() in production Rust. Audit fixes: queue
synchronous=FULL (power-loss durability), ESC/POS band 1024→512
(small printer buffers), duplicate-conflict event log, two Control
Center display nits. External best-practice cross-check (Tauri,
WebView2, Odoo, SQLite, ESC/POS docs + QZ/OCA/Ventor) confirms the
architecture on all six axes, with the divergences deliberate and
documented.

### v1.2.0 (2026-11-03): self-diagnosing webview
- Environment probes on every Odoo page, a boot watchdog that
  replaces silent white pages with a diagnostic panel, one-click
  Clear web data, WebView2 bootstrapper installers, diagnostics
  showing the runtime version. See MEMORY/project history.

## Status (previous): v1.1.1 — Yaseir design system + HTTP origins on private IPs + picker-first UX


### v1.1.0 (2026-11-03): Yaseir identity + LAN HTTP origins
- Control Center rebuilt on the Yaseir design system (ui.yaseir.site
  guide v2.1): Thmanyah Sans bundled locally (dist/fonts, 5 weights,
  @font-face), primary blue #0953DE / brand #0059FF, ink #101219,
  pill buttons (44px), 12px fields, 16-20px cards, 3px focus rings,
  guide type/weight scale, app icons regenerated in the primary blue.
  Copy cut to short SaaS wording in both languages (e.g. settings are
  now simply "Odoo connection" / "الاتصال بـ Odoo"); the product is
  titled Print Gateway / بوابة الطباعة, the family name in the guide.
- Origins (ADR-0002 amendment, owner decision): Odoo may now be
  reached by IP over HTTP - HTTPS for any host, HTTP only for private,
  loopback, or link-local IP literals; hostnames and public IPs still
  require HTTPS. Enforced identically in Rust
  (print-core::security::origin_is_allowed, new unit tests), the app
  config validator, the capability URL patterns, and the Control
  Center's own client-side check.


### v1.1.1 (2026-11-03): stronger UX + new icon
- New app icon: bold printer + check on the Yaseir blue tile (all
  Windows sizes + ICO regenerated), matched by the Control Center
  header mark.
- No typing where a choice exists: the calibration target is now a
  picker fed by discovered printers (with derived tcp:// options and
  a custom-URI fallback), default printer preselected; printer cards
  keep their one-click test buttons.
- Form errors are live: the Odoo address field validates while
  typing - the inline error appears instantly and clears the moment
  the address is valid; toasts already auto-dismiss.
- Verified end to end in a real DOM (jsdom over HTTP, 18/18 checks):
  picker contents, tab switching, live validation, save flow; plus
  18/18 node parity cases for the JS origin rule vs the Rust rule,
  and the full Rust suite green again on this tree.

### Production Completion Summary (2026-11-03)

1. **End-to-End Background Print Worker (`app/src-tauri/src/commands.rs` & `lib.rs`)**:
   - Connected the SQLite WAL queue (`print-core::Queue`) to a dedicated background worker thread (`spawn_worker_loop`) woken immediately via `Condvar` whenever a job is enqueued or retried.
   - Atomic job claiming (`UPDATE jobs SET state='rendering', error_code=NULL, attempts=attempts+1 WHERE id=?1 AND state='queued'`) and automatic payload purging (`purge_payload`) on both `Done` and `Canceled` states so the SQLite database stays compact over years of operation.
   - Implemented `NativePrintBackend` (`print_core::PrintBackend`) supporting:
     - **Windows GDI (`mode = "gdi"`)**: Paper ladder verification (`print_win::paper_forms` + `print_core::match_paper`), `DocumentPropertiesW` 3-step `DEVMODEW` initialization (orientation, matched form / `DMPAPER_USER` custom size, duplex, color), mixed-size sub-job splitting (`split_subjobs`), PDFium rendering with `FPDF_ANNOT | FPDF_PRINTING`, DIB packing, and `StretchDIBits` multi-copy printing (`print_pdf_gdi_with_options`, `print_canvas_gdi`) including `NearestLargerCentered` 100%-scale centering.
     - **Thermal ESC/POS (`mode = "escpos"`)**: Both PDF reports (rendered via PDFium at 203 DPI, scaled to 576/384 dots, thresholded to `MonoBitmap`) and POS PNG captures (`receipt_png_to_escpos`) + `qweb-text` streams, with automatic `1024`-dot horizontal slice chunking (`MAX_RASTER_SLICE_HEIGHT`) so arbitrarily long receipts never overflow printer buffers or `u16` height, dispatched via Windows RAW spooler (`print_win::print_raw`) or direct TCP 9100 (`tcp://host:port`).
     - **Barcode & Shipping Labels (`mode = "zpl" | "tspl" | "epl"`)**: PDF or PNG rendered at 203 DPI (`Threshold` with `FPDF_RENDER_NO_SMOOTHTEXT | NO_SMOOTHIMAGE | NO_SMOOTHPATH` so barcode edges are crisp) and encoded into ZPL II (`zpl_label_with_options`), TSPL2 (`TsplLabel` with `0=black, 1=white` polarity inversion per TSC manual), or EPL2 (`epl_label_copies` with `0=black, 1=white` polarity inversion + `q`/`Q`/`P<copies>`).
     - **Driverless Network IPP (`mode = "ipp"` or `ipp://host:port/path`)**: Queries `Get-Printer-Attributes`, negotiates document format, and submits via `Print-Job`.

2. **Tauri v2 Lifecycle, `AppManifest` ACL & Windows GPO Security**:
   - Registered all 16 app commands in `app/src-tauri/build.rs` via `tauri_build::AppManifest::new().commands(&[...])` so Tauri v2 compile-time ACL enforces `permissions/print.toml` and `capabilities/main.json` on every app command.
   - Hardened `print-core::security::origin_is_allowed` to validate IPv4, IPv6 (`[::1]`), and strict numeric ports, and enforced `validate_job_input` across `print_pdf`, `print_text`, and `print_image`.
   - Fixed window label collision between `tauri.conf.json` and `WebviewWindowBuilder::new(app, "main", ...)` by setting `"windows": []` in `tauri.conf.json` so `.setup()` owns creating `"main"` with `initialization_script(BRIDGE_JS)`.
   - Graceful startup fallback: when `config.json` / GPO has no origin configured yet, the app opens the built-in local **Enterprise Control Center** (`index.html`) instead of crashing on launch.
   - Added Windows Registry Group Policy reading (`HKLM\Software\Policies\OdooNativePrint` -> `OdooOrigin` and `DisableOriginChange`) via `advapi32.dll` (`RegGetValueW`) in `app/src-tauri/src/config.rs`, matching `OdooNativePrint.admx`.
   - Bundled both `resources/pdfium.dll` and `resources/pdfium-LICENSE.txt` in `tauri.conf.json` (`v1.0.0`).

3. **Hardware Protocol & Win32 FFI Hardening**:
   - Fixed ZPL II field order in `crates/print-raw/src/zpl.rs` so `^FO0,0` precedes `^GFA,...` and terminates with `^FS` (`^XA^PW...^LL...^LH...^FO0,0^GFA...^FS^PQ...^XZ`).
   - Fixed TSPL2 (`crates/print-raw/src/tspl.rs`) and EPL2 (`crates/print-raw/src/epl.rs`) binary bitmap polarity (`0 = black dot, 1 = white paper`) to match official TSC TSPL2 and Zebra EPL2 specifications.
   - Upgraded `crates/print-win/src/win.rs` `paper_forms` to query `DC_PAPERNAMES` (16) alongside `DC_PAPERS` (2) and `DC_PAPERSIZE` (3), with `discovery::classify_form` automatically detecting thermal roll forms (`roll: true`) vs cut sheets.

4. **Enterprise UI/UX & Odoo 19 Addon (`odoo/native_print` v19.0.1.2.0)**:
   - **Tauri Control Center (`app/dist/index.html`)**: Full bilingual (Arabic RTL / English LTR) dashboard with dark/light theme, live KPI cards, filterable SQLite job queue (Retry, Cancel, Clear Completed), discovered printer cards with live sensor badges, paper form inspector, one-click calibration test suite (A4 PDF, 80mm/58mm ESC/POS, ZPL, TSPL), HTTPS origin setup, and one-click redacted JSON diagnostics export (`Ctrl+Shift+P` shortcut), with `escapeJsStr` protection for UNC paths (`\\server\printer`) and special characters.
   - **Odoo 19 Addon (`odoo/native_print` v19.0.1.2.0)**:
     - Upgraded views (`views/native_print_views.xml`) with Odoo 19 compliant `<kanban>` (`<t t-name="card">`), `<list>` with badge decorations and inline Retry/Cancel actions, `<form>` with `<header>` statusbar and smart stat buttons, `<search>` with bare `<group>`, `<graph>`, and `<pivot>` views, and `allowed_company_ids` multi-company domain.
     - Updated `native_print_handler.js` to build wizard report URLs (`action.data` -> `?options=...&context=...`), handle `active_id`/`res_id` fallbacks, close wizard modals via `ir.actions.act_window_close` when `close_on_report_download` is set, and sync actual PDF page counts and attempts back to Odoo.
     - Updated `/native_print/rules` controller (`controllers/main.py`) to support catch-all wildcard rules (`*`) and skip archived profiles.
     - Updated `security/native_print_groups.xml` and `ir.model.access.csv` so internal users (`base.group_user`) automatically inherit `group_native_print_user` and can create/update/retry/cancel print jobs out of the box.
     - Added OWL 2 Navbar Systray component (`NativePrintSystray`) and complete Arabic (`i18n/ar.po`) translations.

### Round 2 Adversarial Audit & Stress-Test Hardening (2026-11-03)

1. **PDF Parser & Multi-Page Security (`crates/print-render/src/pdf.rs` & `app/src-tauri/src/commands.rs`)**:
   - Enforced strict PDF token delimiter checks (`has_pdf_token`, `find_pdf_token`, `has_type_token`) so `/PageLabels`, `/PageLayout`, `/PageMode`, and `/Pages` never false-match `/Page`.
   - Added two-pass indirect object resolution for `/MediaBox 5 0 R` and `/Rotate 6 0 R` references, and stripped stream payload bodies prior to dictionary parsing so binary streams containing `/Type /Page` bytes cannot spoof page counts.
   - Validated **all** pages (`for p in &info.pages`) in `commands::print_pdf` against `validate_job_input` rather than only page 0.

2. **58mm (384-Dot) Thermal Roll Calibration & Width Matching (`crates/print-render/src/image.rs`, `calibration.rs`, `commands.rs`)**:
   - Extended `receipt_dots_for_paper` to recognize `"58mm"`, `"58 mm Roll"`, `"58x3276mm"`, and `"57mm"` (`384` dots) in addition to 80mm (`576` dots).
   - Added `calibration_thermal_58mm()` (`384` dots, auto-scaled Code 39 barcode with `unit = 2`) and wired `run_test_print` (`test_type = "escpos_58"` or 58mm paper profiles) to emit true 384-dot test receipts.

3. **ESC/POS Command Injection Sanitization (`crates/print-raw/src/escpos.rs`)**:
   - Hardened `EscPos::text_line` to strip raw control characters (`0x00`, `0x10` DLE, `0x1B` ESC, `0x1C` FS, `0x1D` GS) from untrusted `qweb-text` lines so report text cannot inject arbitrary ESC/POS hardware commands (e.g. cash-drawer kick pulses).

4. **RFC 8010 / RFC 8011 IPP 1.1 & HTTP/1.1 Edge-Case Compliance (`crates/print-ipp/src/proto.rs` & `client.rs`)**:
   - Fixed `parse_message` to append attributes to `groups.last_mut()` when a printer repeats group tags, preventing index mismatch panics, and accepted all RFC 8010 delimiter tags (`0x01..=0x0F`).
   - Updated `client.rs` to treat the full RFC 8011 `0x0000..=0x00FF` range (`successful-ok`, `successful-ok-ignored-or-substituted-attributes`, `successful-ok-conflicting-attributes`) as success, skip interim `HTTP/1.1 100 Continue` status frames, and wrap IPv6 literals (`Host: [::1]:631`) in HTTP `Host` headers.

5. **Win32 `DEVMODEW` Paper IDs & Input Tray (`DC_BINS` / `DC_BINNAMES`) Selection (`crates/print-core/src/paper.rs`, `crates/print-win/src/lib.rs`, `win.rs`)**:
   - Preserved Win32 `DC_PAPERS` `WORD` IDs in `Form::paper_id: Option<u16>` and clamped `paper_forms` iteration to `min(names, sizes, ids)` so drivers returning mismatched capability array counts cannot misalign or panic.
   - Updated `build_printer_devmode` to populate both `dm_paper_size` (`DM_PAPERSIZE`) and `dm_form_name` (`DM_FORMNAME`) when a driver form is matched.
   - Implemented `resolve_printer_bin_id` via `DeviceCapabilitiesW(DC_BINS=6, DC_BINNAMES=12)` (`24-WCHAR` blocks per Microsoft Win32 SDK) to set `dm_default_source` (`DM_DEFAULTSOURCE`) whenever `opts.tray` is specified.

6. **Tauri v2 Dual-Case IPC Keys & Odoo 19 Multi-Company / Systray Hardening (`app/src-tauri/src/lib.rs`, `odoo/native_print`)**:
   - Updated `BRIDGE_JS` to send both `camelCase` and `snake_case` argument keys (`jobId`/`job_id`, `autoReprint`/`auto_reprint`) for compatibility across Tauri v2 command argument conventions.
   - Hardened `/native_print/job_status` (`controllers/main.py`) with `_safe_int` parsing, field length caps (`2000` chars on `error_message`, `256` chars on `printer_name`/`report_name`), and same-company `profile_id` ownership verification.
   - Added window outside-click listener (`onMounted` / `onWillDestroy`) in `native_print_systray.js` so the OWL 2 systray popover closes cleanly when clicking anywhere outside.

## Principal Engineering Mission (2026-11-03) — main tree audit & repair
Full report: `docs/engineering-mission-2026-11-03.md`. Headlines:
- CRITICAL fixed: sync_printers crashed on every call (UnboundLocalError,
  zero controller tests); repeat sync blind after archive sweep
  (active_test=False); Odoo 19 ignores _sql_constraints — device/job
  unique indexes did not exist, now models.Constraint + addon
  19.0.1.3.0 pre-migration dedupe; origin-policy fork (HTTP-only/IP-only,
  HTTPS rejected) unified to HTTPS-any-host + HTTP-any-IP per the
  owner's recorded ADR-0002 decision.
- HIGH fixed: fake job actions removed + state-write guard +
  owner-scoped job_status + job record rules; desktop idempotency
  payload overwrite (enqueue_new); ambiguous submission failures no
  longer auto-retry (BackendError::ambiguous); unknown print modes
  fail loud; client routing aligned to server tiers.
- MEDIUM fixed: terminal-state monotonicity, scale_policy passthrough,
  profile counters, forms JSON truncation, JS status-watch windows,
  remote capability least-privilege split, paper-override roll check,
  odoo.yml runs on odoo/** changes, bridge version = package version.
- Verified here: Rust 90 + print-win 8 tests, clippy/fmt clean,
  PDFium render 4/4, bench/chaos PASS, cargo-deny ok, Odoo 19 suite
  26/26 green on a fresh DB with both unique indexes present,
  Control Center 9/9 + jsdom DOM policy check.
- Still gated: Windows CI on push, signing certificate, field spikes.

## Zero-config mission (2026-11-03)
- Sync now finishes setup itself (`_ensure_auto_setup`): a profile per
  synced printer + a first-time catch-all rule to the default physical
  printer (priority 0), because regular users are read-only on
  profiles/rules. Idempotent; hand-modelled config outranks it.
  Profile create() accepts the controller's auto-setup context and
  validates against the device owner.
- Rule payload carries a compact `media` block (roll width from the
  driver forms + form count); handler and POS pass it to the bridge
  as `mediaRollWidthMm`.
- Desktop: JobOptions.media_roll_width_mm; TCP/IPP raw printing fits
  receipts to the server-reported roll width when no local driver
  exists; label modes fit rendered bitmaps into the largest
  driver-reported label form via print-render::fit_mono_to_box
  (downscale only, 8-dot aligned, unit-tested).
- Addon 19.0.1.4.0. Verified: Odoo suite 28/28 on a fresh DB,
  workspace 93 tests + clippy clean, Control Center 9/9, JS syntax ok.
  Docs: docs/zero-config.md.

## Routing & interception hardening (2026-11-03, research-driven)
- Handler fallbacks: report-fetch failures and bridge submission
  failures now fall back to Odoo's standard download with a warning
  toast instead of throwing (job still logged failed server-side).
- One-click default printer: /native_print/set_default_printer
  re-points the user's catch-all; /native_print/overview reports the
  routing default; the systray lists the desktop's printers with
  Set default / star badge. Addon 19.0.1.5.0.
- docs/routing-and-interception.md records the benchmarked design
  (OCA, Ventor, PrintNode, QZ, IoT) and the rejected alternatives.
- Verified: Odoo suite 29/29 on a fresh DB; JS syntax ok.

## Live printer model — simplification (2026-11-03, owner decision)
- Removed ALL server-side printer storage: native.print.device /
  .profile / .rule models, their views, menus, ACLs and record
  rules are gone; the endpoints /native_print/sync_printers,
  /native_print/rules and /native_print/set_default_printer are
  gone (a test asserts they 404). The server keeps only the job
  log (job_status + overview counters). Addon 19.0.2.0.0.
- Printing resolves live in the page from the desktop bridge:
  systray-selected printer (browser storage only) -> POS prefers
  thermal/label -> Windows default -> first printer. Capabilities
  and media are read from the driver at click time; protocol is
  inferred client-side from the driver identity (the former server
  detection table, ported to native_print_client.js); raw printers
  on an IP port print via tcp://<ip>:9100, others via their queue.
- Verified: Odoo suite 11/11 fresh DB exit 0 (job log + controller
  tests incl. removed-route 404s); node live-target tests pass
  (detection, patterns, selection order, stale-selection fallback).

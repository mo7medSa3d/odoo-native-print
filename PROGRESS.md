# PROGRESS

Last updated: 2026-11-03 (Africa/Cairo)
Owner spec: Master Build Prompt — Odoo Native Print Client (Windows, then Android, then iOS)

## Status: v1.0.0 Production Release Ready — Complete End-to-End Pipeline, Worker Execution, GPO Support, Protocol Hardening & Enterprise UI/UX

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

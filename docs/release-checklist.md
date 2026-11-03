# Release Checklist — v1.0.0 Production Release

## Verified in Repository & Automated Test Suite
- [x] `cargo test --workspace`: all unit and integration tests passing (`print-core`, `print-render`, `print-raw`, `print-ipp`, `print-win`, `app/src-tauri`)
- [x] Tauri v2 ACL command gating: all 16 app commands registered in `app/src-tauri/build.rs` via `tauri_build::AppManifest::new().commands(&[...])` and mapped in `permissions/print.toml` + `capabilities/main.json`
- [x] Exact-origin IPC validation in Rust (`print_core::security::origin_is_allowed`) with IPv4, IPv6 (`[::1]`), port, and scheme checks + `check_local_only` on `save_origin_cmd`
- [x] Durable SQLite WAL queue (`print-core::queue`): atomic `claim_next` (`UPDATE ... WHERE state='queued'`), payload purging on `Done` and `Canceled`, crash recovery (`recover_on_startup`, ADR-0004), and backpressure limit (`500` active jobs/printer)
- [x] PDFium dynamic FFI (`print-render::pdfium`): `FPDF_ANNOT | FPDF_PRINTING` render flags (plus `FPDF_RENDER_NO_SMOOTHTEXT | NO_SMOOTHIMAGE | NO_SMOOTHPATH` in 1-bit `Threshold` mode), safe struct drop ordering (`api` before `_lib`), and 128 MiB per-page ceiling
- [x] Win32 GDI & Spooler backend (`print-win`): `DocumentPropertiesW` 3-step `DEVMODEW` initialization (orientation, matched form / `DMPAPER_USER` custom size, duplex, color), mixed-size PDF sub-job splitting (`split_subjobs`), `NearestLargerCentered` 100%-scale centering, and `DC_PAPERS`/`DC_PAPERSIZE`/`DC_PAPERNAMES` roll detection
- [x] Raw printer protocols (`print-raw`): ESC/POS `GS v 0` with automatic `1024`-dot slice chunking for long receipts, ZPL II (`^XA^PW...^LL...^LH...^FO0,0^GFA...^FS^PQ...^XZ`), TSPL2 (`BITMAP` with `0=black, 1=white` polarity inversion), EPL2 (`GW` with `0=black, 1=white` polarity inversion + `q`/`Q`/`P<copies>`), and TCP 9100 + `DLE EOT` status sensor
- [x] Driverless IPP 1.1 client (`print-ipp`): `Get-Printer-Attributes` format negotiation and `Print-Job` submission without duplicate `document-format` attributes (CUPS #1457)
- [x] Odoo 19 addon (`odoo/native_print` v19.0.1.2.0):
  - Report handler (`native_print_handler.js`) supporting `qweb-pdf` and `qweb-text`, wizard `action.data` (`?options=...&context=...`), `active_id`/`res_id` fallbacks, and `close_on_report_download` modal closing via `ir.actions.act_window_close`
  - POS receipt interceptor (`pos_native_print.js`) supporting Odoo 18/19 (`PosStore.prototype.printReceipt`) and Odoo 17 (`ReceiptScreen.prototype.printReceipt`), 2x SVG `foreignObject` PNG capture, and status polling
  - OWL 2 Navbar Systray widget (`NativePrintSystray`) + Odoo 19 compliant Kanban (`<t t-name="card">`), List, Form, Search, Graph, and Pivot views
  - Multi-company rules (`allowed_company_ids`), default internal user group assignment (`base.group_user` -> `group_native_print_user`), and complete Arabic translations (`i18n/ar.po`)
- [x] Bilingual Enterprise Control Center (`app/dist/index.html`): XSS/JS-escaped printer and job actions (`escapeJsStr`), live KPI cards, filterable job queue, calibration lab, GPO status, and redacted diagnostics export

## Field & Hardware Gates (Run on Target Windows / Odoo 19 Site)
- [ ] Windows CI job (`windows-tests`, `windows-app`, `windows-print-smoke`) on GitHub Actions
- [ ] Spikes S1 (live Tauri WebView2 IPC negative origin tests), S2/S2b (live Odoo 19 report & POS flows), S4 (50 MB PDF transfer timing), S5 (`Microsoft Print to PDF` at A4/A5/80mm/100x150mm)
- [ ] Hardware validation kit (`docs/hardware/validation-kit.md`): one signed PASS sheet per physical printer class
- [ ] Production code-signing certificate wired into `.github/workflows/release.yml` for MSI/NSIS installers

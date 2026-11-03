# Odoo Native Print Client (v1.0.0)

A production-grade native client (**Tauri 2**, **Rust** core, embedded **PDFium**) and **Odoo 19** addon that renders and prints Odoo reports, shipping labels, kitchen tickets, and Point of Sale (POS) receipts directly on local and LAN printers. No gateway, no local background agent, no localhost HTTP daemon, and no cloud relay in the print path.

## Architecture & Repository Layout

- **`crates/print-core`** — Durable SQLite/WAL queue (`busy_timeout` concurrent-safe), `JobOptions` persistence, strict job state machine (`queued -> rendering -> spooling -> printing -> done | failed | canceled | dead_letter`), crash recovery (ADR-0004), exponential backoff worker engine, fail-loud paper matching ladder (ADR-0003), printer status sensor decoding, exact-origin IPC security validation (ADR-0002), and redacted diagnostics bundle generator.
- **`crates/print-render`** — Dynamic PDFium bindings (`libloading`, pinned Chromium/8125 `158.0.8125.0`), PDF inspection parser, memory-bounded render planner (128 MiB/page ceiling), mixed-size sub-job splitter, defensive PNG decoder & area-average receipt downscaler (`80mm` -> 576 dots, `58mm` -> 384 dots), and built-in A4 PDF + 80mm thermal calibration generators.
- **`crates/print-raw`** — Raw printer language encoders and transports: ESC/POS (`GS v 0` raster, feed, cut, drawer kick), ZPL II (`^XA^PW...^LL...^LH...^FO0,0^GFA...^FS^PQ...^XZ`), TSPL (`SIZE`/`GAP`/`CLS`/`BITMAP`/`PRINT`), EPL2 (`GW`), 1-bit `MonoBitmap`, and TCP 9100 (JetDirect) transport with `DLE EOT` real-time status queries.
- **`crates/print-ipp`** — Dependency-free IPP 1.1 client (RFC 8010/8011) with `Get-Printer-Attributes` format negotiation (`application/pdf`, `image/pwg-raster`, `image/urf`, `image/jpeg`) and `Print-Job` submission.
- **`crates/print-win`** — Win32 spooler & GDI backend (`winspool.drv` + `gdi32.dll`): printer enumeration (`EnumPrintersW` level 2), live status sensors (`GetPrinterW` level 2), paper form discovery (`DeviceCapabilitiesW` `DC_PAPERS`, `DC_PAPERSIZE`, and `DC_PAPERNAMES` with automatic roll paper detection), RAW spooler submission, and GDI bitmap printing (`CreateDCW` / `StartDocW` / `StretchDIBits` over PDFium & PNG DIBs with multi-copy support).
- **`app/`** — Tauri v2 desktop application with:
  - Background print worker thread (`spawn_worker_loop` + `NativePrintBackend`) connecting the durable SQLite queue to Win32 GDI, Win32 RAW spooler, TCP 9100, and IPP printers.
  - Windows Group Policy (`HKLM\Software\Policies\OdooNativePrint` via `advapi32.dll` `RegGetValueW`) and `config.json` origin resolution.
  - Built-in bilingual (Arabic RTL / English LTR) **Enterprise Control Center & Calibration Lab** (`app/dist/index.html`, accessible anytime via `Ctrl+Shift+P` or when no origin is configured yet).
- **`odoo/native_print`** — Odoo 19 addon (`19.0.1.2.0`):
  - Printer Profiles, Routing Rules, and Print Jobs log with Odoo 19 Kanban (`<t t-name="card">`), List, Form, Search, Graph, and Pivot views.
  - OWL 2 Navbar **Systray Widget** (`NativePrintSystray`) showing live bridge connection state, local printer count, job counters, one-click calibration print, and quick navigation.
  - Report handler (`ir.actions.report handlers`) for `qweb-pdf` and `qweb-text` with background job status polling and toast notifications.
  - POS Receipt interceptor (`point_of_sale._assets_pos`) supporting both Odoo 18/19 (`PosStore.prototype.printReceipt`) and Odoo 17 (`ReceiptScreen.prototype.printReceipt`), capturing the on-screen receipt DOM as a 2x PNG and printing via ESC/POS raster or GDI.
  - Complete Arabic (`i18n/ar.po`) translations.
- **`docs/`** — ADRs 0001–0004, API Contract v1, STRIDE threat model, hardware validation kit, Odoo setup guide, and operations runbooks.

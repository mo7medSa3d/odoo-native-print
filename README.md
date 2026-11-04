# Odoo Native Print Client (v1.4.12)

A production-grade native client (**Tauri 2**, **Rust** core, embedded **PDFium**) and **Odoo 19** addon (**19.0.2.15.0**) that renders and prints Odoo reports, shipping labels, kitchen tickets, and Point of Sale (POS) receipts directly on local and LAN printers. No gateway, no local background agent, no localhost HTTP daemon, and no cloud relay in the print path.

## Architecture & Repository Layout

- **`crates/print-core`** — Durable SQLite/WAL queue (`busy_timeout` concurrent-safe), `JobOptions` persistence, strict job state machine (`queued -> rendering -> spooling -> printing -> done | failed | canceled | dead_letter`, with `failed`/`dead_letter` retryable back to `queued`), crash recovery (ADR-0004), durable retry backoff (`available_at`, never a worker-thread sleep), fail-loud paper matching ladder (ADR-0003), multi-condition printer health model (source, timestamp, TTL), stable printer identity, capability provenance, exact-origin IPC security validation (ADR-0002), and redacted diagnostics bundle generator.
- **`crates/print-render`** — Dynamic PDFium bindings (`libloading`, pinned Chromium/8125 `158.0.8125.0`), PDF inspection parser, memory-bounded render planner (128 MiB/page ceiling), mixed-size sub-job splitter, defensive PNG decoder & area-average receipt downscaler (203-dpi head profiles: `80mm` -> 576 dots, `58mm` -> 384 dots, scaled by the driver-reported render resolution at the app layer), and built-in A4 PDF + 80mm thermal calibration generators.
- **`crates/print-raw`** — Raw printer language encoders and transports: ESC/POS (`GS v 0` raster, feed, cut, drawer kick), ZPL II (`^XA^PW...^LL...^LH...^FO0,0^GFA...^FS^PQ...^XZ`), TSPL (`SIZE`/`GAP`/`CLS`/`BITMAP`/`PRINT`, gap from the job option), EPL2 (`GW`, gap from the job option), 1-bit `MonoBitmap`, TCP 9100 (JetDirect) transport with `DLE EOT` real-time status queries, plus library-only status parsers (DLE EOT n=1–n=4, TSPL `ESC !?`, ZPL `~HS`/SGD) that are not wired into print paths.
- **`crates/print-ipp`** — Dependency-free IPP 1.1 client (RFC 8010/8011) with `Get-Printer-Attributes` negotiation (**PDF only** — no raster encoder exists) and `Print-Job` submission, including the blocking-reasons interpretation (`is_blocked()`).
- **`crates/print-win`** — Win32 spooler & GDI backend (`winspool.drv` + `gdi32.dll`): printer enumeration (`EnumPrintersW` level 2), live multi-condition health reads (`GetPrinterW` level 2), paper form discovery (`DeviceCapabilitiesW` `DC_PAPERS`, `DC_PAPERSIZE`, and `DC_PAPERNAMES` with automatic roll paper detection; failed queries reported as unknown provenance, never fabricated), RAW spooler submission (write loop + abort-on-error), and GDI bitmap printing (`CreateDCW` / `StartDocW` / `StretchDIBits` over PDFium & PNG DIBs with multi-copy support).
- **`app/`** — Tauri v2 desktop application with:
  - Background print worker thread (`spawn_worker_loop` + `NativePrintBackend`) connecting the durable SQLite queue to Win32 GDI, Win32 RAW spooler, TCP 9100, and IPP printers.
  - Windows Group Policy (`HKLM\Software\Policies\OdooNativePrint` via `advapi32.dll` `RegGetValueW`) and `config.json` origin resolution.
  - Built-in bilingual (Arabic RTL / English LTR) **Enterprise Control Center & Calibration Lab** (`app/dist/index.html`, accessible anytime via `Ctrl+Shift+Y` or when no origin is configured yet), showing printer health conditions, sources, and staleness.
- **`odoo/native_print`** — Odoo 19 addon:
  - Job log (`native.print.job`) and history-only printer registry (`native.print.device`) with Odoo 19 views.
  - OWL 2 Navbar **Systray Widget** (`NativePrintSystray`) showing live bridge connection state, local printer count, job counters, one-click calibration print, and quick navigation.
  - Report handler (`ir.actions.report` handlers) for `qweb-pdf` and `qweb-text` with background job status polling and toast notifications; failed jobs are watched through retries so the log converges on the final outcome.
  - POS Receipt interceptor (`point_of_sale._assets_pos`) supporting Odoo 19 (`PosStore.prototype.printReceipt`), capturing the on-screen receipt DOM as a 2x PNG and printing via ESC/POS raster or GDI.
  - Complete Arabic (`i18n/ar.po`) translations.
- **`docs/`** — ADRs 0001–0004, API Contract v1, STRIDE threat model, hardware validation kit, Odoo setup guide, and operations runbooks. `PRINTER_COMPATIBILITY_MATRIX.md` (repo root) is the protocol/sensor truth table; `ENGINEERING_AUDIT.md` and `VERIFICATION_REPORT.md` describe this cycle's evidence.

## Key semantics

- **Queue:** one durable SQLite queue; job + payload + options land in one transaction; duplicate idempotency keys return the original job untouched.
- **Ambiguity:** a job whose physical outcome is unknown is never silently auto-reprinted. Recovery distinguishes proven-not-submitted from ambiguous.
- **Status honesty:** a spooler acceptance, a TCP write, or an IPP acceptance is not proof paper emerged. Sensor data is shown with its source and age; unknown stays unknown.
- **Fallback:** with no desktop bridge, Odoo keeps its standard download/print behavior.

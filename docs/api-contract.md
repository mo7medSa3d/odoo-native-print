# API Contract v1: Odoo Addon <-> Native Print App

Versioned. Breaking changes bump the major version on both sides.

## Bridge (injected by the Tauri shell into the main window)

### Core Print Commands (Odoo Addon -> Native App)

- **`window.__NATIVE_PRINT__.printPdf({ idempotencyKey, reportName, printerPattern, mode, paper, copies, duplex, color, scalePolicy, url, bytes })`**
  - `bytes`: `Uint8Array`, `ArrayBuffer`, `number[]`, or Base64 string of the PDF document (automatically normalized by the bridge and accepted by Rust's multi-format `deserialize_bytes`).
  - Limits enforced in Rust (`print-core::security`): `<= 50 MiB`, `<= 2,000 pages`, `copies 1..=99`, page dimensions `<= 1,500 mm`.
  - Queued durably in SQLite WAL with `JobOptions` and immediately dispatched by the background worker to GDI (`print-win`), ESC/POS raster, ZPL/TSPL/EPL, TCP 9100, or IPP.

- **`window.__NATIVE_PRINT__.printText({ idempotencyKey, reportName, printerPattern, text, cut })`**
  - `qweb-text` reports (kitchen tickets). Encoded to ESC/POS text lines + feed + partial cut and queued with priority `10`.

- **`window.__NATIVE_PRINT__.printImage({ idempotencyKey, reportName, printerPattern, mode, paper, copies, bytes })`**
  - `bytes`: PNG image bytes captured by the POS receipt extension (`point_of_sale.receipt`).
  - Validated at the door via `print_render::decode_png` (`<= 20 MiB`, `<= 20,000 px` side, `<= 50M` pixels), queued with priority `10`, and printed by the worker via ESC/POS raster (`80mm` -> 576 dots, `58mm` -> 384 dots), ZPL/TSPL/EPL, or GDI.

### Status, Discovery & Control Center Commands

- **`window.__NATIVE_PRINT__.printStatus(jobId)`** -> `Promise<string>` (`"queued" | "rendering" | "spooling" | "printing" | "done" | "failed" | "canceled" | "dead_letter"`)
- **`window.__NATIVE_PRINT__.jobDetail(jobId)`** -> `Promise<{ id, state, errorCode, printer, report, pages, bytes, attempts }>`
- **`window.__NATIVE_PRINT__.appOverview()`** -> `Promise<AppOverviewResponse>` (version, OS, configured origin, GPO lock status, PDFium status, discovered printers, default printer, state counts, recent 200 jobs)
- **`window.__NATIVE_PRINT__.listPrinters()`** -> `Promise<PrinterInfo[]>`
- **`window.__NATIVE_PRINT__.printerForms(printer)`** -> `Promise<Form[]>`
- **`window.__NATIVE_PRINT__.runTestPrint(printer, kind, paper)`** -> `Promise<string>` (`kind`: `"pdf-a4" | "thermal-80mm" | "thermal-58mm" | "zpl-label" | "tspl-label"`)
- **`window.__NATIVE_PRINT__.retryJob(jobId)`** -> `Promise<void>`
- **`window.__NATIVE_PRINT__.cancelJob(jobId)`** -> `Promise<void>`
- **`window.__NATIVE_PRINT__.clearFinished()`** -> `Promise<number>`
- **`window.__NATIVE_PRINT__.exportDiagnostics()`** -> `Promise<string>` (redacted JSON bundle)
- **`window.__NATIVE_PRINT__.saveOrigin(origin, autoReprint)`** -> `Promise<string>` (callable only from the local Control Center origin and only when not locked by Windows Group Policy)
- **`window.__NATIVE_PRINT__.openControlCenter()` / `openOdooOrigin()`** -> toggles between the Odoo webview and the built-in Control Center (`Ctrl+Shift+P`).

## Server Endpoints (Odoo Addon Controllers)

- **`POST /native_print/rules`** (`type="jsonrpc"`, `auth="user"`): resolves the highest-priority active rule for `report_name` within `request.env.company` and the caller's security groups.
- **`POST /native_print/job_status`** (`type="jsonrpc"`, `auth="user"`): creates or updates the company-scoped `native.print.job` audit log row by `idempotency_key`.
- **`POST /native_print/overview`** (`type="jsonrpc"`, `auth="user"`): returns server-side profile, rule, and job counts for the OWL Systray widget.

# API Contract v1: Odoo Addon <-> Native Print App

Versioned. Breaking changes bump the major version on both sides.

## Bridge (injected by the Tauri shell into the main window)
Origin policy (ADR-0002, owner decision 2026-11-03): the configured
origin must match exactly. HTTPS is accepted for any host; HTTP is
accepted for any IP literal, private or public (hostnames over HTTP
are rejected). HTTP is unencrypted; the Control Center warns about it.

### Core Print Commands (Odoo Addon -> Native App)

- **`window.__NATIVE_PRINT__.printPdf({ idempotencyKey, reportName, printerPattern, mode, paper, copies, duplex, color, scalePolicy, url, bytes })`**
  - `bytes`: `Uint8Array`, `ArrayBuffer`, `number[]`, or Base64 string of the PDF document (automatically normalized by the bridge and accepted by Rust's multi-format `deserialize_bytes`).
  - Limits enforced in Rust (`print-core::security`): `<= 50 MiB`, `<= 2,000 pages`, `copies 1..=99`, page dimensions `<= 1,500 mm`.
  - Queued durably in SQLite WAL with `JobOptions` and immediately dispatched by the background worker to GDI (`print-win`), ESC/POS raster, ZPL/TSPL/EPL, TCP 9100, or IPP.

- **`window.__NATIVE_PRINT__.printText({ idempotencyKey, reportName, printerPattern, text, cut })`**
  - `qweb-text` reports (kitchen tickets). Encoded to ESC/POS text lines + feed + partial cut and queued with priority `10`.

- **`window.__NATIVE_PRINT__.printImage({ idempotencyKey, reportName, printerPattern, mode, copies, bytes })`** (legacy `paper` is optional; new POS jobs omit it so the native worker resolves the roll width from the driver)
  - `bytes`: PNG image bytes captured by the POS receipt extension (`point_of_sale.receipt`).
  - Validated at the door via `print_render::decode_png` (`<= 20 MiB`, `<= 20,000 px` side, `<= 50M` pixels), queued with priority `10`, and printed by the worker via ESC/POS raster (`80mm` -> 576 dots, `58mm` -> 384 dots), ZPL/TSPL/EPL, or GDI.

### Status, Discovery & Control Center Commands

- **`window.__NATIVE_PRINT__.printStatus(jobId)`** -> `Promise<string>` (`"queued" | "rendering" | "spooling" | "printing" | "done" | "failed" | "canceled" | "dead_letter"`)
- **`window.__NATIVE_PRINT__.jobDetail(jobId)`** -> `Promise<{ id, state, errorCode, printer, report, pages, bytes, attempts }>`
- **`window.__NATIVE_PRINT__.appOverview()`** -> `Promise<AppOverviewResponse>` (version, OS, configured origin, GPO lock status, PDFium status, discovered printers, default printer, state counts, recent 200 jobs)
- **`window.__NATIVE_PRINT__.listPrinters()`** -> `Promise<PrinterInfo[]>`
- **`window.__NATIVE_PRINT__.printerForms(printer)`** -> `Promise<Form[]>`
- **`window.__NATIVE_PRINT__.printerCapabilities(printer)`** -> `Promise<PrinterCapabilities>` (paper forms, trays, resolutions, duplex/color, custom-size bounds and copy limit)
- **`window.__NATIVE_PRINT__.runTestPrint(printer, kind, paper)`** -> `Promise<string>` (`kind`: `"pdf-a4" | "thermal-80mm" | "thermal-58mm" | "zpl-label" | "tspl-label"`)
- **`window.__NATIVE_PRINT__.retryJob(jobId)`** -> `Promise<void>`
- **`window.__NATIVE_PRINT__.cancelJob(jobId)`** -> `Promise<void>`
- **`window.__NATIVE_PRINT__.clearFinished()`** -> `Promise<number>`
- **`window.__NATIVE_PRINT__.exportDiagnostics()`** -> `Promise<string>` (redacted JSON bundle)
- **`window.__NATIVE_PRINT__.saveOrigin(origin, autoReprint)`** -> `Promise<string>` (callable only from the local Control Center origin and only when not locked by Windows Group Policy)
- **`window.__NATIVE_PRINT__.openControlCenter()` / `openOdooOrigin()`** -> toggles between the Odoo webview and the built-in Control Center (`Ctrl+Shift+P`).

## Desktop printer discovery and routing (live model)

Printers are never stored on the server (owner decision 2026-11-03:
no printer records, no profiles, no cross-PC sharing). The desktop
app on the user's own PC is the only printer authority. At click
time the Odoo page asks the bridge for THIS PC's current printers,
picks the target live (the printer the user selected in the systray
— kept only in browser storage — else the OS default; POS receipts
prefer a thermal/label printer), reads that printer's real
driver-reported capabilities (forms, roll width, duplex, color)
through the bridge, infers the print mode from the driver identity,
and hands the job to the desktop. A printer that is not connected
right now can never be routed to. The server keeps only the job
log.
capabilities. Protocol inference is conservative: only recognized driver
signatures select ESC/POS, ZPL, TSPL or EPL; unknown devices stay on the
Windows driver-managed GDI path. Company managers can inspect the full
company printer inventory across workstations, including human-readable forms,
roll dimensions, input bins, resolutions and device protocol; actual routing
remains bound to the exact desktop UUID and user. Network queues installed in
Windows remain on the Windows spooler/GDI route unless a separate direct IPP
endpoint is explicitly configured.

Profiles do not request a paper type, paper size, tray, duplex or color choice.
The Windows driver is the source of truth. PDF jobs use an exact reported form,
a custom size only within the driver's reported bounds, or the smallest larger
compatible form; if none is safe, the job fails with a media error. POS/kitchen
thermal jobs use the receipt width mapped from the driver's detected roll
form. Unsupported roll widths are rejected rather than guessed as 58mm/80mm.

## Server Endpoints (Odoo Addon Controllers)

- **`POST /native_print/job_status`** (`type="jsonrpc"`, `auth="user"`): creates or updates the company-scoped, owner-scoped `native.print.job` audit log row by `idempotency_key`. Terminal states are final.
- **`POST /native_print/overview`** (`type="jsonrpc"`, `auth="user"`): returns job counters for the OWL Systray widget.

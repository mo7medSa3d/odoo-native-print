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
  - Queued durably in SQLite WAL with `JobOptions` and immediately dispatched by the background worker to GDI (`print-win`), ESC/POS raster, ZPL/TSPL/EPL/CPCL, TCP 9100, or IPP.

- **`window.__NATIVE_PRINT__.printText({ idempotencyKey, reportName, printerPattern, text, cut })`**
  - `qweb-text` reports (kitchen tickets). `text` must be non-empty and `<= 1 MiB` (`1,048,576` bytes; rejected with `bad_text_size` otherwise). Encoded to ESC/POS text lines + feed + partial cut and queued with priority `10`.

- **`window.__NATIVE_PRINT__.printImage({ idempotencyKey, reportName, printerPattern, mode, copies, bytes })`** (legacy `paper` is optional; new POS jobs omit it so the native worker resolves the roll width from the driver)
  - `bytes`: PNG image bytes captured by the POS receipt extension (`point_of_sale.receipt`).
  - Validated at the door via `print_render::decode_png` (`<= 20 MiB`, `<= 40,000 px` side, `<= 50M` pixels), queued with priority `10`, and printed by the worker via ESC/POS raster (`80mm` -> 576 dots, `58mm` -> 384 dots), ZPL/TSPL/EPL/CPCL, or GDI.

### Status, Discovery & Control Center Commands

- **`window.__NATIVE_PRINT__.printStatus(jobId)`** -> `Promise<string>` (`"queued" | "rendering" | "spooling" | "printing" | "done" | "failed" | "canceled" | "dead_letter"`)
- **`window.__NATIVE_PRINT__.jobDetail(jobId)`** -> `Promise<{ id, state, errorCode, printer, report, pages, bytes, attempts }>`
- **`window.__NATIVE_PRINT__.appOverview()`** -> `Promise<AppOverviewResponse>` (version, OS, configured origin, GPO lock status, PDFium status, **WebView2 runtime version**, **latest environment probe**, discovered printers, default printer, state counts, recent 200 jobs)
- **`window.__NATIVE_PRINT__.listPrinters()`** -> `Promise<PrinterInfo[]>`
- **`window.__NATIVE_PRINT__.printerForms(printer)`** -> `Promise<Form[]>`
- **`window.__NATIVE_PRINT__.printerCapabilities(printer)`** -> `Promise<PrinterCapabilities>` (paper forms, trays, resolutions, duplex/color, custom-size bounds and copy limit)
- **`window.__NATIVE_PRINT__.runTestPrint(printer, kind, paper, options)`** -> `Promise<string>` (`kind`: `"pdf-a4" | "thermal-80mm" | "thermal-58mm" | "escpos" | "zpl-label" | "zpl" | "tspl-label" | "tspl" | "epl-label" | "epl" | "cpcl-label" | "cpcl"`)
- **`window.__NATIVE_PRINT__.retryJob(jobId)`** -> `Promise<void>`
- **`window.__NATIVE_PRINT__.cancelJob(jobId)`** -> `Promise<void>`
- **`window.__NATIVE_PRINT__.clearFinished()`** -> `Promise<number>`
- **`window.__NATIVE_PRINT__.exportDiagnostics()`** -> `Promise<string>` (redacted JSON bundle)
- **`window.__NATIVE_PRINT__.saveOrigin(origin, autoReprint)`** -> `Promise<string>` (callable only from the local Control Center origin and only when not locked by Windows Group Policy)
- **`window.__NATIVE_PRINT__.getPrinterSettings()` / `setPrinterSettings(printer, settings)`** -> `Promise<PrinterSettingsMap>` (local Control Center only; reads/updates per-printer paper/offset/calibration overrides)
- **`window.__NATIVE_PRINT__.openControlCenter()` / `openOdooOrigin()`** -> toggles between the Odoo webview and the built-in Control Center (`Ctrl+Shift+Y`).
- **`window.__NATIVE_PRINT__.tabOpen(url)`** -> `Promise<string>` (opens an on-origin URL in a new tab; validated in Rust against the configured origin).
- **`window.__NATIVE_PRINT__.reportEnvironment(report)`** -> `Promise<void>` — the bridge's own environment probe (JS level, storage, cookies, WebSocket, secure context) reported on every Odoo page load; surfaced in `appOverview().environment` and in the diagnostics bundle. Not called by addon code directly.
- **`window.__NATIVE_PRINT__.clearWebData()`** -> `Promise<void>` — wipes the app's whole browsing profile (cache, cookies, storage, service workers) and reloads the configured origin. The self-heal for a poisoned profile / white-page failures; the user signs in again afterwards.

### Complete Tauri Command Surface (32 commands in `app/src-tauri/src/commands.rs`)

In addition to the `window.__NATIVE_PRINT__` facade above, the local tab bar (`tabbar.html`), boot page (`boot.html`), and settings page (`settings.html`) invoke shell/tab commands directly via Tauri IPC. Permissions are enforced in `app/src-tauri/permissions/print.toml` and `capabilities/{main,remote,toolbar}.json`:

| ACL Group | Capabilities | Rust Commands |
| --- | --- | --- |
| `allow-print-pdf` (3) | `main`, `remote` | `print_pdf`, `print_text`, `print_image` |
| `allow-print-status` (7) | `main`, `remote` | `print_status`, `job_detail`, `app_overview`, `list_printers`, `printer_forms`, `printer_capabilities`, `report_environment` |
| `allow-app-shell` (3) | `main`, `remote` | `open_control_center`, `open_settings`, `tab_open` |
| `allow-queue-admin` (7) | `main` only | `retry_job_cmd`, `cancel_job_cmd`, `clear_finished_cmd`, `export_diagnostics`, `save_origin_cmd`, `get_printer_settings`, `set_printer_settings` |
| `allow-local-tools` (2) | `main` only | `run_test_print`, `probe_origin` |
| `allow-web-data-reset` (1) | `main` only | `clear_web_data_cmd` |
| `allow-settings-nav` (2) | `main` only | `app_identity`, `open_odoo_origin` |
| `allow-toolbar` (12) | `toolbar` only | `nav_back`, `nav_forward`, `nav_reload`, `tab_open`, `tab_list`, `tab_switch`, `tab_move`, `tab_close`, `app_identity`, `open_control_center`, `open_settings`, `open_odoo_origin` |

Submission guarantees (desktop queue): a job, its payload and its
options are written in a single SQLite transaction; a duplicate
idempotency key returns the original job and writes nothing. Queue
backpressure: 500 active jobs per printer and 2000 globally —
beyond that, submission is refused instead of consuming unbounded
disk. Jobs carry a diagnostics id (`np-...`, a stable fingerprint of
the idempotency key) shown in job details and the diagnostics
bundle, whose `recent_jobs` summaries and `recent_events` log are
metadata-only by construction.

## Desktop printer discovery and routing (live model)

Routing is never driven by server-side profiles or rules (no profiles,
no routing rules, no cross-PC sharing). The desktop app on the user's
own PC is the only live printer authority. At click time the Odoo page
asks the bridge for THIS PC's current printers, picks the target live
(the printer the user selected in the systray — kept only in browser
storage — else the OS default; POS receipts prefer a thermal/label
printer), reads that printer's real driver-reported capabilities (forms,
roll width, duplex, color) through the bridge, infers the print mode
from the driver identity, and hands the job to the desktop. A printer
that is not connected right now can never be routed to. The server keeps
the operational job log (`native.print.job`) and an auto-synced
history/picker registry (`native.print.device`).
Protocol inference is conservative: only recognized driver
signatures select ESC/POS, ZPL, TSPL, EPL or CPCL; unknown devices stay on the
Windows driver-managed GDI path. Network queues installed in
Windows remain on the Windows spooler/GDI route unless a separate direct IPP
endpoint is explicitly configured.

Paper type, paper size, tray, duplex and color choices follow the
Windows driver as the source of truth. PDF jobs use an exact reported form,
a custom size only within the driver's reported bounds, or the smallest larger
compatible form; if none is safe, the job fails with a media error. POS/kitchen
thermal jobs use the receipt width mapped from the driver's detected roll
form. Unsupported roll widths are rejected rather than guessed as 58mm/80mm.

## Server Endpoints (Odoo Addon Controllers)

- **`POST /native_print/job_status`** (`type="jsonrpc"`, `auth="user"`): creates or updates the company-scoped, owner-scoped `native.print.job` audit log row by `idempotency_key`. A formal state machine governs updates: states only move forward (`queued -> rendering -> spooling -> printing -> terminal`; forward skips are legal, backward moves are answered `ignored: "illegal_transition"`, a printing job cannot be canceled), terminal states are final (`ignored: "terminal_state_is_final"`), and `attempts` never regresses.
- **`POST /native_print/sync_device`** (`type="jsonrpc"`, `auth="user"`): upserts the history-only `native.print.device` picker registry record for a workstation printer seen by the desktop bridge.
- **`POST /native_print/overview`** (`type="jsonrpc"`, `auth="user"`): returns job counters for the OWL Systray widget.

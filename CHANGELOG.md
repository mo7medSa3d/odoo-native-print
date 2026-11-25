# Changelog

All notable changes to **Odoo Native Print** are documented in this file.
The desktop client (`app/`), the Odoo 19 addon (`odoo19/native_print/`), and the Odoo 20 addon (`odoo20/native_print/`) follow independent version lines.

## Desktop Client `1.5.7` / Odoo 19 Addon `19.0.2.24.0` / Odoo 20 Addon `20.0.1.0.0` — 2026-11-20

### Desktop Client (`1.5.7`)
- **Core Queue & Worker (`print-core`):** Durable SQLite/WAL print queue with atomic claiming, transactional payload + options persistence, idempotency deduplication, durable `available_at` retry backoff, crash recovery without silent auto-reprint of ambiguous jobs (ADR-0004), and 500 active-job per-printer backpressure.
- **Rendering (`print-render`):** Dynamic PDFium FFI pinned to Chromium/8125 (`158.0.8125.0`), 128 MiB per-page memory ceiling, mixed-size PDF sub-job splitting, area-average 203-dpi receipt downscaling (`80mm` / `58mm`), and built-in A4/80mm calibration generators.
- **Printers & Protocols (`print-win`, `print-raw`, `print-ipp`):** Win32 GDI (`DEVMODEW` 3-step init, `StretchDIBits`, `DeviceCapabilitiesW` form/roll discovery) and RAW spooler submission; ESC/POS (`GS v 0` chunked raster, cut, drawer kick), ZPL II, TSPL2, EPL2, CPCL, TCP 9100 + `DLE EOT` status, and dependency-free IPP 1.1 PDF printing.
- **Security & Enterprise Ops (`app/`):** Tauri v2 ACL command gating across all 32 commands, exact-origin IPC validation (ADR-0002), Windows GPO (`HKLM\Software\Policies\OdooNativePrint`) origin enforcement, redacted diagnostics export, and bilingual (Arabic RTL / English LTR) Enterprise Control Center (`Ctrl+Shift+Y`).

### Odoo 19 Addon (`19.0.2.24.0`)
- **Models & Views:** Per-company job log (`native.print.job`) and auto-synced history-only printer registry (`native.print.device`) with Odoo 19 Kanban, List, Form, Search, Graph, and Pivot views.
- **Report & POS Interception:** `qweb-pdf` and `qweb-text` report handler with background job status polling and retryable-outcome convergence; POS receipt interceptor (`point_of_sale._assets_pos`) capturing 2x receipt PNGs for ESC/POS or GDI printing; automatic fallback to standard browser printing when outside the desktop client.
- **UI & Localization:** OWL 2 navbar systray widget (`NativePrintSystray`) with live bridge status, job counters, and calibration shortcut; complete Arabic translations (`i18n/ar.po`); multi-company record rules.

### Odoo 20 Addon (`20.0.1.0.0`)
- **Odoo 20 Port (`odoo20/native_print/`):** Native Odoo 20 addon with `security/ir.access.csv`, dedicated dark-mode stylesheet (`native_print_dark.scss`), OWL systray widget, report & POS receipt interception, multi-company job/device logs, and Arabic translations (`i18n/ar.po`).

### Delivery & Release Packaging
- Added parameterized `Dockerfile.odoo` (non-root `odoo:19.0` / `20.0` via `ODOO_VERSION` and `ADDON_SRC` build args with `/web/health` check) and streamlined operational documentation.

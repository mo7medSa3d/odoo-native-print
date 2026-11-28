# Changelog

All notable changes to **Itbaa (اطبع)** are documented in this file.
The desktop client (`app/`), Android companion (`android-app/`), and the Odoo 17–20 addons (`odoo17/native_print/` through `odoo20/native_print/`) follow independent version lines.

## Itbaa (اطبع) Rebrand, Offline Ed25519 Licensing & Direct-Distribution Hardening — 2026-11-28

### Brand & Distribution
- **Itbaa (اطبع) Rebrand:** Renamed desktop client (`productName: "Itbaa"`), Android companion (`ItbaaApp`), Odoo 17–20 addon labels/translations, static site, and operational docs to **Itbaa (اطبع)** while retaining existing Windows GPO (`HKLM\Software\Policies\OdooNativePrint`), per-user config, and Odoo technical module (`native_print`) compatibility.
- **Cloudflare Pages Site (`website/`):** Added static bilingual marketing site with strict `website/_headers` security policy (`Content-Security-Policy` with `script-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`, HSTS, `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`, immutable caching for `/fonts/*` and `/assets/*`), externalized `website/theme-init.js` (zero inline executable scripts), and `website/robots.txt`.

### Offline Ed25519 Licensing (`ITBAA1.*`)
- **Local Verification:** Implemented offline `< 1 ms` Ed25519 signature verification (`ITBAA1.<Base64URL(json)>.<Base64URL(sig)>`) bound to exact normalized Odoo origins with clock-rollback protection and zero external activation servers.
- **Issuance Tooling:** Added `tools/license_keygen.py` (`keygen`, `sign`, `verify`) and `.github/workflows/generate-license.yml` (`workflow_dispatch`) backed by `ITBAA_LICENSE_PRIVATE_KEY`, with automatic license redaction in diagnostics.

### $0 Direct-Distribution Hardening
- **Windows Desktop (`app/`, `crates/`):** Protected stored licenses with Windows DPAPI (`CryptProtectData`), XOR-assembled embedded Ed25519 public key (`assemble_public_key`), 3-point license verification gate, `withGlobalTauri: false`, and stripped/path-remapped release builds (`debug = false`, `strip = true`, `--remap-path-prefix`).
- **Android Companion (`android-app/`):** Encrypted license storage with `AndroidKeyStore` (`AES/GCM/NoPadding`), APK signing-certificate SHA-256 anti-tampering (`ApkIntegrityGuard.verifyCertDigest`), R8 fullMode (`android.enableR8.fullMode=true`) with `android.util.Log` stripping, `android:allowBackup="false"`, `isDebuggable = false`, and `setWebContentsDebuggingEnabled(BuildConfig.DEBUG)`.
- **Verification Gates (`tools/verify_local.py`):** Added automated checks for Ed25519 test vectors, brand/secret/public-download hygiene, 14/14 release hardening controls, and website `_headers`/CSP/SEO compliance.

## Odoo 17 (`17.0.1.0.0`) & Odoo 18 (`18.0.1.0.0`) Addons + Release Hardening — 2026-11-28

### Odoo 17 & 18 Addons (`17.0.1.0.0`, `18.0.1.0.0`)
- **Multi-Version Odoo Support (`odoo17/native_print/`, `odoo18/native_print/`):** Added native Odoo 17 and Odoo 18 addons with full backend QWeb (`qweb-pdf` / `qweb-text`) and POS receipt interception, OWL navbar systray widget, multi-company job/device logs, dark-mode stylesheet, and Arabic translations (`i18n/ar.po`), matching the `window.__NATIVE_PRINT__` v1 contract across Windows desktop and Android clients.

### Security, Reliability & CI Hardening
- **Expanded CI Coverage:** Added `odoo17` and `odoo18` to `.github/workflows/odoo.yml` (Docker install + `--test-tags /native_print` matrix across `17.0`–`20.0`) and `.github/workflows/ci.yml` (Python compile, XML validation, JS ESM syntax check, and Node unit tests).
- **Real-DOM PR Gate:** Added `app/tests/jsdom` (`npm ci && node --test *.test.mjs`) to `.github/workflows/ci.yml` so Control Center and bridge DOM behavior tests run on every PR.
- **Release Hygiene & Docs:** Removed internal AI scratch plan (`docs/superpowers/`), documented `ODOO_VERSION` (`17.0`–`20.0`) and `ADDON_SRC` pairs in `.env.example`, and tightened `README.md` for operational setup, testing, and deployment across Odoo 17–20, Windows desktop, and Android.

### Verification
- Validated all `.github/workflows/*.yml` files with `yaml.safe_load` and ran `python3 tools/verify_local.py` with zero regressions.

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

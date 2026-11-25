# Odoo Native Print

Zero-gateway local printing for **Odoo 19** (`19.0.2.24.0`, `odoo19/native_print/`) and **Odoo 20** (`20.0.1.0.0`, `odoo20/native_print/`) powered by a **Tauri 2 / Rust** desktop client (`1.5.7`) with embedded **PDFium** (`158.0.8125.0`). Prints QWeb reports, shipping labels (ZPL II, TSPL2, EPL2, CPCL), kitchen tickets, and POS receipts (ESC/POS, GDI, IPP) directly to local and LAN printers—no local HTTP daemon, no IoT box, and no cloud relay in the print path.

## Requirements

- **Windows Client (Production):** Windows 11 (or Windows 10 with Microsoft Edge WebView2 Runtime) and installed Windows printer drivers for target printers. `pdfium.dll` is bundled in the installer.
- **Odoo Server:** Odoo `19.0` (`odoo19/native_print/`) or Odoo `20.0` (`odoo20/native_print/`) with PostgreSQL `16+` (Docker or Linux server).
- **Local Development / CI:** Rust `1.90.0`, Node.js `20+`, Python `3.11+` (stdlib only for the local verifier).

## Setup

1. Copy the environment template and set real credentials before running Docker (never commit `.env` or signing certificates):
   ```bash
   cp .env.example .env
   ```
2. *(Linux dev/CI only)* Fetch the pinned PDFium library (`158.0.8125.0`) for `print-render` tests:
   ```bash
   bash tools/fetch-pdfium.sh
   ```
3. Install desktop JS dependencies when building the Tauri shell:
   ```bash
   cd app && npm ci
   ```
4. Install `odoo19/native_print` (Odoo 19) or `odoo20/native_print` (Odoo 20) on your Odoo instance (see [Odoo Setup](docs/odoo-setup.md)).

## Configuration

See [`.env.example`](.env.example) for the full variable list and defaults.

| Variable / Setting | Scope | Default / Format | Production Guidance |
| :--- | :--- | :--- | :--- |
| `HKLM\Software\Policies\OdooNativePrint` | Windows GPO | `OdooOrigin` (`https://...`) | **Recommended** for fleet deployment; overrides user config. |
| `%APPDATA%\OdooNativePrint\config.json` | Desktop app | JSON (`origin`) | Per-user origin when GPO is not applied. |
| `NATIVE_PRINT_ODOO_ORIGIN` | Dev env var | `https://erp.example.com` | **Dev only**; do not set in production. |
| `NATIVE_PRINT_PDFIUM_PATH` | Dev/CI env var | Path to `pdfium.dll` / `libpdfium.so` | Leave unset in production (bundled in MSI/NSIS). |
| `PDFIUM_TEST_REQUIRED` | CI env var | `1` | Fails `print-render` tests if PDFium is missing. |
| `ODOO_VERSION`, `ADDON_SRC` | `Dockerfile.odoo` | `19.0`, `odoo19/native_print` | Set to `20.0` and `odoo20/native_print` for Odoo 20 builds. |
| `ODOO_PORT`, `HOST`, `PORT`, `USER`, `PASSWORD` | Odoo Docker | `8069`, `db`, `5432`, `odoo`, secret | Inject `PASSWORD` via secret manager / Docker secrets. |
| `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD` | Postgres Docker | `postgres`, `odoo`, secret | Match `USER` / `PASSWORD` used by Odoo. |
| `WINDOWS_CERTIFICATE_BASE64`, `WINDOWS_CERTIFICATE_PASSWORD` | GitHub Secrets | Base64 `.pfx` + password | Required in `.github/workflows/release.yml` for signed installers. |
| **Odoo Navbar Systray & Native Print > Jobs** | Odoo 19 / 20 UI | Live workstation printer picker | Printers are discovered live on each workstation; POS prefers thermal/label printers automatically. |

## Run

- **Odoo 19 (Docker, default):**
  ```bash
  docker build -f Dockerfile.odoo -t odoo-native-print:19.0 .
  docker run -d --name odoo-native-print \
    -p "${ODOO_PORT:-8069}:8069" \
    --env-file .env \
    odoo-native-print:19.0
  ```
- **Odoo 20 (Docker):**
  ```bash
  docker build -f Dockerfile.odoo \
    --build-arg ODOO_VERSION=20.0 \
    --build-arg ADDON_SRC=odoo20/native_print \
    -t odoo-native-print:20.0 .
  docker run -d --name odoo-native-print \
    -p "${ODOO_PORT:-8069}:8069" \
    --env-file .env \
    odoo-native-print:20.0
  ```
- **Desktop Client (Windows):**
  ```powershell
  cd app
  npm run build
  ```
  Press `Ctrl+Shift+Y` inside the desktop client to open the bilingual (Arabic RTL / English LTR) **Enterprise Control Center & Calibration Lab**.

## Test

Run the offline local verifier (version consistency, command surface, bash syntax, Node tests, Python compile, XML/ADMX/ADML, TOML/JSON, and `ar.po` checks) and its self-test:

```bash
python3 tools/verify_local.py
python3 tools/verify_local.py --self-test
```

Additional CI suites:
- **Rust workspace:** `cargo test --locked --workspace` and `cargo clippy --locked --workspace --all-targets -- -D warnings`
- **PDFium render tests:** `PDFIUM_TEST_REQUIRED=1 cargo test --locked -p print-render --test pdfium_render`
- **Odoo 19 & 20 addon tests:** Automated in `.github/workflows/odoo.yml` against `odoo:19.0` and `odoo:20.0` + `postgres:16.4` (`--test-tags /native_print`).

## Deploy

- **Windows Fleet (MSI / NSIS + GPO):**
  1. Tag a release (`v1.5.7`) to build MSI and NSIS installers + `SHA256SUMS.txt` via `.github/workflows/release.yml` (Authenticode-signed when `WINDOWS_CERTIFICATE_*` secrets are set).
  2. Deploy the MSI silently via Intune/SCCM/GPO and apply the bundled ADMX/ADML policy templates to enforce `OdooOrigin`.
  3. Complete field verification per [Release Checklist](docs/release-checklist.md) and [Hardware Validation Kit](docs/hardware/validation-kit.md).
- **Odoo 19 / 20 Server / Docker:**
  1. Build and run `Dockerfile.odoo` (non-root `odoo` user, `/mnt/extra-addons/native_print`, built-in `HEALTHCHECK` on `http://127.0.0.1:8069/web/health`) for Odoo 19 (default) or Odoo 20 (`--build-arg ODOO_VERSION=20.0 --build-arg ADDON_SRC=odoo20/native_print`), or copy `odoo19/native_print` (Odoo 19) / `odoo20/native_print` (Odoo 20) into your server's `addons_path`.
  2. Update the Apps list, install **Yaseir Print** (`native_print`), assign **Native Print / User** and **Native Print / Manager** groups, and select workstation printers live from the navbar systray.

## Documentation

- [Architecture Decision Records](docs/adr/) (`0001`–`0006`)
- [API Contract](docs/api-contract.md) & [Error Catalog](docs/error-catalog.md)
- [Odoo Setup Guide](docs/odoo-setup.md) & [Operations Runbooks](docs/runbooks.md)
- [Security Threat Model](docs/security/threat-model.md)
- [PDFium Bundling](docs/pdfium.md), [CI Workflows](docs/ci.md), & [Release Checklist](docs/release-checklist.md)
- [Changelog](CHANGELOG.md)


## Android Companion App (`android-app/`)

`android-app/` is the native Kotlin + Jetpack Compose companion browser and silent printing app for Android phones and POS tablets (`minSdk 26`, `targetSdk 35`), matching the Windows desktop app's `window.__NATIVE_PRINT__` v1 contract, Light/Dark theme, and `Thmanyah Sans` typography with zero changes required in `odoo19/native_print` or `odoo20/native_print`.

- **Printers supported:** TCP 9100 raw (`ESC/POS`, `ZPL`, `TSPL`, `EPL`, `CPCL`), IPP/Mopria (`mDNS/NSD`), Classic Bluetooth SPP (`RFCOMM`), USB Host OTG (Printer Class `0x07`), and Android `PrintManager` fallback.
- **Build & Test:** Open `android-app/` in Android Studio or run:
  ```bash
  cd android-app
  gradle :app:testDebugUnitTest :app:assembleDebug
  ```

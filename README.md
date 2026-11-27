# Odoo Native Print

Zero-gateway silent printing for **Odoo 17** (`17.0.1.0.0`), **Odoo 18** (`18.0.1.0.0`), **Odoo 19** (`19.0.2.24.0`), and **Odoo 20** (`20.0.1.0.0`), powered by a **Windows Tauri 2 / Rust** desktop client (`1.5.7`, bundled **PDFium** `158.0.8125.0`) and a native **Android** Kotlin / Jetpack Compose companion app (`android-app/`). Prints QWeb reports, shipping labels (ZPL II, TSPL2, EPL2, CPCL), kitchen tickets, and POS receipts (ESC/POS, GDI, IPP, Bluetooth SPP, USB OTG) directly from workstation and POS devices—no local HTTP daemon, no IoT box, and no cloud relay in the print path.

## Requirements

- **Windows Desktop Client (Production):** Windows 11 (or Windows 10 with Microsoft Edge WebView2 Runtime) and installed Windows printer drivers for target printers. `pdfium.dll` is bundled in the installer.
- **Android Companion App:** Android `8.0+` (`minSdk 26`, `targetSdk 35`) for TCP 9100, IPP/Mopria (`mDNS/NSD`), Classic Bluetooth SPP (`RFCOMM`), USB Host OTG (Printer Class `0x07`), or `PrintManager` fallback.
- **Odoo Server:** Odoo `17.0` (`odoo17/native_print/`), `18.0` (`odoo18/native_print/`), `19.0` (`odoo19/native_print/`), or `20.0` (`odoo20/native_print/`) with PostgreSQL `16+`.
- **Local Development / CI:** Rust `1.90.0`, Node.js `20+`, Python `3.11+` (stdlib only for `tools/verify_local.py`), and JDK 17 / Gradle for Android builds.

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
4. Install the matching addon (`odoo17/native_print`, `odoo18/native_print`, `odoo19/native_print`, or `odoo20/native_print`) on your Odoo instance (see [Odoo Setup](docs/odoo-setup.md)). Both desktop and Android clients implement the same `window.__NATIVE_PRINT__` v1 contract with no addon changes required.

## Configuration

See [`.env.example`](.env.example) for the full variable list and defaults.

| Variable / Setting | Scope | Default / Format | Production Guidance |
| :--- | :--- | :--- | :--- |
| `HKLM\Software\Policies\OdooNativePrint` | Windows GPO | `OdooOrigin` (`https://...`) | **Recommended** for fleet deployment; overrides per-user config. |
| `%APPDATA%\OdooNativePrint\config.json` | Windows desktop | JSON (`origin`) | Per-user Odoo origin when GPO is not applied. |
| `NATIVE_PRINT_ODOO_ORIGIN` | Dev env var | `https://erp.example.com` | **Dev only**; do not set in production. |
| `NATIVE_PRINT_PDFIUM_PATH` | Dev/CI env var | Path to `pdfium.dll` / `libpdfium.so` | Leave unset in production (bundled in MSI/NSIS). |
| `PDFIUM_TEST_REQUIRED` | CI env var | `1` | Fails `print-render` tests if PDFium is missing. |
| `ODOO_VERSION`, `ADDON_SRC` | `Dockerfile.odoo` | `19.0`, `odoo19/native_print` | Pair `17.0`–`20.0` with `odoo17`–`odoo20/native_print`. |
| `ODOO_PORT`, `HOST`, `PORT`, `USER`, `PASSWORD` | Odoo Docker | `8069`, `db`, `5432`, `odoo`, secret | Inject `PASSWORD` via secret manager / Docker secrets. |
| `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD` | Postgres Docker | `postgres`, `odoo`, secret | Match `USER` / `PASSWORD` used by Odoo. |
| `WINDOWS_CERTIFICATE_BASE64`, `WINDOWS_CERTIFICATE_PASSWORD` | GitHub Secrets | Base64 `.pfx` + password | Required in `.github/workflows/release.yml` for signed installers. |
| **Odoo Navbar Systray & Native Print > Jobs** | Odoo 17–20 UI | Live workstation printer picker | Printers are discovered live per device; POS prefers thermal/label printers automatically. |

## Run

- **Odoo 17–20 (Docker):** Set `ODOO_VERSION` (`17.0`–`20.0`) and `ADDON_SRC` (`odoo17`–`odoo20/native_print`; defaults to `19.0`):
  ```bash
  docker build -f Dockerfile.odoo \
    --build-arg ODOO_VERSION=19.0 \
    --build-arg ADDON_SRC=odoo19/native_print \
    -t odoo-native-print:19.0 .
  docker run -d --name odoo-native-print \
    -p "${ODOO_PORT:-8069}:8069" \
    --env-file .env \
    odoo-native-print:19.0
  ```
- **Windows Desktop Client:**
  ```powershell
  cd app && npm run build
  ```
  Press `Ctrl+Shift+Y` inside the app to open the bilingual (Arabic RTL / English LTR) **Enterprise Control Center & Calibration Lab**.
- **Android Companion App:** Open `android-app/` in Android Studio or build from CLI:
  ```bash
  cd android-app && gradle :app:assembleDebug
  ```

## Test

- **Offline local verifier & self-test** (versions, command surface, bash syntax, Node tests across Odoo 17–20, Python compile, XML/ADMX/ADML, TOML/JSON, `ar.po`):
  ```bash
  python3 tools/verify_local.py
  python3 tools/verify_local.py --self-test
  ```
- **Real-DOM JSDOM tests (Control Center & bridge):**
  ```bash
  cd app/tests/jsdom && npm ci && node --test *.test.mjs
  ```
- **Rust workspace & PDFium render tests:**
  ```bash
  cargo clippy --locked --workspace --all-targets -- -D warnings
  cargo test --locked --workspace
  PDFIUM_TEST_REQUIRED=1 cargo test --locked -p print-render --test pdfium_render
  ```
- **Android unit tests:**
  ```bash
  cd android-app && gradle :app:testDebugUnitTest
  ```
- **Odoo 17–20 Docker addon tests:** Automated in `.github/workflows/odoo.yml` against `odoo:17.0`, `odoo:18.0`, `odoo:19.0`, and `odoo:20.0` + `postgres:16.4` (`--test-tags /native_print`).

## Deploy

- **Odoo 17–20 Server / Docker:**
  1. Build and run `Dockerfile.odoo` (non-root `odoo` user, `/mnt/extra-addons/native_print`, built-in `HEALTHCHECK` on `http://127.0.0.1:8069/web/health`) with the matching `ODOO_VERSION` / `ADDON_SRC`, or copy `odoo17..20/native_print` into your server's `addons_path`.
  2. Update Apps, install **Yaseir Print** (`native_print`), assign **Native Print / User** and **Native Print / Manager** groups, and pick printers live from the navbar systray.
- **Windows Fleet (MSI / NSIS + GPO):**
  1. Tag a release (`v1.5.7`) to build MSI and NSIS installers + `SHA256SUMS.txt` via `.github/workflows/release.yml` (Authenticode-signed when `WINDOWS_CERTIFICATE_*` secrets are configured).
  2. Deploy the MSI silently via Intune/SCCM/GPO and apply the bundled ADMX/ADML policy templates to enforce `OdooOrigin`.
  3. Complete field verification per [Release Checklist](docs/release-checklist.md) and [Hardware Validation Kit](docs/hardware/validation-kit.md).
- **Android Fleet:** Build release APKs/AABs from `android-app/` (automated CI checks run in `.github/workflows/android.yml`) and distribute via MDM or Google Play.

## Documentation

- [Architecture Decision Records](docs/adr/) (`0001`–`0006`)
- [API Contract](docs/api-contract.md) & [Error Catalog](docs/error-catalog.md)
- [Odoo Setup Guide](docs/odoo-setup.md) & [Operations Runbooks](docs/runbooks.md)
- [Security Threat Model](docs/security/threat-model.md)
- [PDFium Bundling](docs/pdfium.md), [CI Workflows](docs/ci.md), & [Release Checklist](docs/release-checklist.md)
- [Changelog](CHANGELOG.md)

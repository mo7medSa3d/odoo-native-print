# Itbaa (اطبع)

Zero-gateway silent printing for **Odoo 17** (`17.0.1.0.0`), **Odoo 18** (`18.0.1.0.0`), **Odoo 19** (`19.0.2.24.0`), and **Odoo 20** (`20.0.1.0.0`), powered by a **Windows Tauri 2 / Rust** desktop client (`1.5.7`, bundled **PDFium** `158.0.8125.0`), a native **Android** Kotlin / Jetpack Compose companion app (`android-app/`), offline **Ed25519** licensing (`ITBAA1.*`), and a static **Cloudflare Pages** site (`website/`). Prints QWeb reports, shipping labels (ZPL II, TSPL2, EPL2, CPCL), kitchen tickets, and POS receipts (ESC/POS, GDI, IPP, Bluetooth SPP, USB OTG) directly from workstation and POS devices—no local HTTP daemon, no IoT box, and no cloud relay in the print path.

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
| `HKLM\Software\Policies\OdooNativePrint` | Windows GPO | `OdooOrigin`, `LicenseKey`, `DisableOriginChange` | **Recommended** for fleet deployment; overrides per-user config and preserves license across updates. |
| `%APPDATA%\OdooNativePrint\config.json` | Windows desktop | JSON (`origin`) | Per-user Odoo origin when GPO is not applied. |
| `NATIVE_PRINT_ODOO_ORIGIN` | Dev env var | `https://erp.example.com` | **Dev only**; do not set in production. |
| `NATIVE_PRINT_PDFIUM_PATH` | Dev/CI env var | Path to `pdfium.dll` / `libpdfium.so` | Leave unset in production (bundled in MSI/NSIS). |
| `PDFIUM_TEST_REQUIRED` | CI env var | `1` | Fails `print-render` tests if PDFium is missing. |
| `ODOO_VERSION`, `ADDON_SRC` | `Dockerfile.odoo` | `19.0`, `odoo19/native_print` | Pair `17.0`–`20.0` with `odoo17`–`odoo20/native_print`. |
| `ODOO_PORT`, `HOST`, `PORT`, `USER`, `PASSWORD` | Odoo Docker | `8069`, `db`, `5432`, `odoo`, secret | Inject `PASSWORD` via secret manager / Docker secrets. |
| `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD` | Postgres Docker | `postgres`, `odoo`, secret | Match `USER` / `PASSWORD` used by Odoo. |
| `WINDOWS_CERTIFICATE_BASE64`, `WINDOWS_CERTIFICATE_PASSWORD` | GitHub Secrets | Base64 `.pfx` + password | Required in [`.github/workflows/release.yml`](.github/workflows/release.yml) for signed installers. |
| `ITBAA_LICENSE_PRIVATE_KEY` | GitHub Secret / CLI env | 32-byte Ed25519 seed (hex or Base64URL) | Required for `tools/license_keygen.py sign` and [`.github/workflows/generate-license.yml`](.github/workflows/generate-license.yml); never commit. |
| `ITBAA_ANDROID_KEYSTORE_BASE64`, `ITBAA_ANDROID_KEYSTORE_PASSWORD`, `ITBAA_ANDROID_KEY_ALIAS`, `ITBAA_ANDROID_KEY_PASSWORD` | GitHub Secrets | Base64 `.jks`, store password, key alias, key password | Required in [`.github/workflows/android.yml`](.github/workflows/android.yml) for signed Release APKs. |
| **Odoo Navbar Systray & Native Print > Jobs** | Odoo 17–20 UI | Live workstation printer picker | Printers are discovered live per device; POS prefers thermal/label printers automatically. |

## Licensing (Offline Ed25519)

Itbaa verifies activation keys locally in `< 1 ms` using Ed25519 signatures with no external activation server (`ITBAA1.<Base64URL(json)>.<Base64URL(sig)>`), bound to the exact normalized Odoo origin:

- **Generate a keypair (once, store private key only in GitHub Secrets):**
  ```bash
  python3 tools/license_keygen.py keygen
  ```
- **Sign a customer license locally (or run [`.github/workflows/generate-license.yml`](.github/workflows/generate-license.yml) manually):**
  ```bash
  ITBAA_LICENSE_PRIVATE_KEY="<64-hex-or-b64url>" python3 tools/license_keygen.py sign \
    --customer "Acme Retail" --origin "https://erp.acme.com" --days 365 --tier pro
  ```
  Pass `--days 0` for a perpetual license.
- **Verify a license key offline:**
  ```bash
  python3 tools/license_keygen.py verify --key "ITBAA1...." --pubkey "<public-key>" --origin "https://erp.acme.com"
  ```

## Run

- **Odoo 17–20 (Docker):** Set `ODOO_VERSION` (`17.0`–`20.0`) and `ADDON_SRC` (`odoo17`–`odoo20/native_print`; defaults to `19.0`):
  ```bash
  docker build -f Dockerfile.odoo \
    --build-arg ODOO_VERSION=19.0 \
    --build-arg ADDON_SRC=odoo19/native_print \
    -t itbaa-odoo:19.0 .
  docker run -d --name itbaa-odoo \
    -p "${ODOO_PORT:-8069}:8069" \
    --env-file .env \
    itbaa-odoo:19.0
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

- **Offline local verifier & self-test** (versions, command surface, bash syntax, Node tests across Odoo 17–20, website CSP/headers, Python compile, XML/ADMX/ADML, TOML/JSON, `ar.po`, Ed25519 vectors, brand/secrets, and release hardening):
  ```bash
  python3 tools/verify_local.py
  python3 tools/verify_local.py --self-test
  ```
- **Website JS syntax check:**
  ```bash
  node --check website/app.js website/theme-init.js
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
- **Odoo 17–20 Docker addon tests:** Automated in [`.github/workflows/odoo.yml`](.github/workflows/odoo.yml) against `odoo:17.0`, `odoo:18.0`, `odoo:19.0`, and `odoo:20.0` + `postgres:16.4` (`--test-tags /native_print`).

## Deploy

- **Odoo 17–20 Server / Docker:**
  1. Build and run [`Dockerfile.odoo`](Dockerfile.odoo) (non-root `odoo` user, `/mnt/extra-addons/native_print`, built-in `HEALTHCHECK` on `http://127.0.0.1:8069/web/health`) with the matching `ODOO_VERSION` / `ADDON_SRC`, or copy `odoo17..20/native_print` into your server's `addons_path`.
  2. Update Apps, install **Itbaa** (`native_print`), assign **Native Print / User** and **Native Print / Manager** groups, and pick printers live from the navbar systray.
- **Windows Fleet (MSI / NSIS + GPO):**
  1. Tag a release (`v1.5.7`) to build hardened MSI and NSIS installers + `SHA256SUMS.txt` via [`.github/workflows/release.yml`](.github/workflows/release.yml) (Authenticode-signed when `WINDOWS_CERTIFICATE_*` secrets are configured).
  2. Deploy the MSI silently via Intune/SCCM/GPO and apply the bundled ADMX/ADML policy templates to enforce `OdooOrigin` and `LicenseKey`.
  3. Complete field verification per [Release Checklist](docs/release-checklist.md) and [Hardware Validation Kit](docs/hardware/validation-kit.md).
- **Android Fleet:** Configure the `ITBAA_ANDROID_KEYSTORE_*` / `ITBAA_ANDROID_KEY_*` secrets in GitHub Actions to build R8-hardened, signed Release APKs + `SHA256SUMS.txt` via [`.github/workflows/android.yml`](.github/workflows/android.yml), then distribute via MDM or direct install.
- **Cloudflare Pages (`website/`):**
  1. Connect the repository in Cloudflare Pages with **Build command:** *(empty)* and **Build output directory:** `website` — or deploy directly from CLI:
     ```bash
     npx wrangler pages deploy website --project-name itbaa
     ```
  2. Cloudflare Pages automatically applies [`website/_headers`](website/_headers) (strict CSP with `script-src 'self'`, HSTS, `X-Frame-Options: DENY`, `nosniff`, and 1-year immutable caching for `/fonts/*` and `/assets/*`) alongside [`website/robots.txt`](website/robots.txt).

## Documentation

- [Architecture Decision Records](docs/adr/) (`0001`–`0006`)
- [API Contract](docs/api-contract.md) & [Error Catalog](docs/error-catalog.md)
- [Odoo Setup Guide](docs/odoo-setup.md) & [Operations Runbooks](docs/runbooks.md)
- [Security Threat Model](docs/security/threat-model.md)
- [PDFium Bundling](docs/pdfium.md), [CI Workflows](docs/ci.md), & [Release Checklist](docs/release-checklist.md)
- [Changelog](CHANGELOG.md)

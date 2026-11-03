# App shell (Tauri 2) — Windows desktop application (`v1.0.0`)

Production Tauri v2 desktop shell (`app/src-tauri`) and bilingual Arabic/English **Enterprise Control Center & Calibration Lab** (`app/dist/index.html`).

On a Windows runner or developer workstation:

```powershell
cd app
npm ci
npm run build
```

## Connection policy

The desktop client accepts Odoo only as `http://<IPv4-or-IPv6>[:port]`.
The Rust origin guard accepts HTTPS for any host and HTTP for any IP literal (hostnames over HTTP are rejected), browser
validator, navigation handler, and Tauri remote capability. HTTP is
unencrypted; use a private network/VPN and test accounts only.

## Security & Architecture Highlights

- **`withGlobalTauri: false`** — no global `__TAURI__` object leaks to web pages; only the hardened `window.__NATIVE_PRINT__` bridge is injected via `WebviewWindowBuilder::initialization_script`.
- **Tauri v2 `AppManifest::commands` Enforcement (`build.rs`)** — all 16 app commands are registered in `tauri_build::AppManifest::new().commands(&[...])`, so Tauri's compile-time ACL strictly gates every command using `permissions/print.toml` and `capabilities/main.json`.
- **Defense-in-Depth HTTP/IP Origin Lock (ADR-0002)** — every Rust command independently validates `webview.url()` via `print_core::security::origin_is_allowed` (mitigating CVE-2026-42184 origin confusion). Only the exact configured origin is accepted: HTTPS for any host, or HTTP to an IP address (ADR-0002). `save_origin_cmd` remains restricted to the local Control Center origin (`check_local_only`).
- **Windows Group Policy (`deploy/admx/OdooNativePrint.admx`)** — reads `HKLM\Software\Policies\OdooNativePrint` (`OdooOrigin` and `DisableOriginChange`) via `advapi32.dll` (`RegGetValueW`).
- **No `fs`, `shell`, `http`, or `opener` plugins** — zero file-system or shell execution surface exposed to the webview.
- **Background Worker & Hardware Dispatch** — `spawn_worker_loop` drains the durable SQLite WAL queue through `NativePrintBackend` across Windows GDI (`DocumentPropertiesW` / `DEVMODEW` + `StretchDIBits` + sub-job splitting), Windows RAW Spooler, ESC/POS, ZPL II, TSPL2, EPL2, TCP 9100, and IPP 1.1.

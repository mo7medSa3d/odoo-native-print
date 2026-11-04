# Compatibility Policy

What works with what, and how mismatches behave. The rule everywhere:
**a version mismatch must fail loudly and helpfully, never print
garbage or fail silently.**

## Supported matrix

| Component | Supported | Notes |
| --- | --- | --- |
| Odoo | **19.0** (addon `native_print` 19.0.x) | The addon uses Odoo 19 APIs (e.g. `models.Constraint`, `res.groups.user_ids`). Older Odoo versions are not supported. |
| Desktop app | Windows 10 (22H2) and Windows 11, x64 | WebView2 runtime required; the installer brings it current via the embedded bootstrapper (internet at install time, or an already-current runtime). |
| WebView2 runtime | Evergreen (bootstrapper-updated) | The app reports the exact runtime version in the Control Center diagnostics tab. |
| Printers | Any printer with a Windows driver (GDI), plus raw ESC/POS, ZPL, TSPL, EPL devices via spooler RAW or direct TCP 9100 / IPP | Capabilities are read from the driver at print time; nothing is stored server-side. |

## App ↔ addon pairing (API v1)

The desktop app and the Odoo addon speak **bridge API v1**
(`docs/api-contract.md`): the `window.__NATIVE_PRINT__` bridge, the
`/native_print/job_status` and `/native_print/overview` endpoints.

- The addon works without the app: in a normal browser, reports and
  POS receipts keep Odoo's standard behaviour (download / browser
  print). Nothing breaks; printing is simply not intercepted.
- The app works with any API-v1 addon. Newer addons may use more of
  the bridge; every bridge method is capability-checked by the page
  (`typeof bridge.x === "function"`) before use, so a newer addon
  degrades gracefully on an older app.
- An older addon on a newer app keeps working: bridge methods are
  additive only within v1. A method is never removed or changed
  incompatibly in v1.
- Unknown print modes fail with `UNKNOWN_MODE` on the job — the
  signal to align the pair — instead of sending bytes in a guessed
  format.

## Versioning

- App: semantic `MAJOR.MINOR.PATCH` in `tauri.conf.json`,
  `Cargo.toml`, `package.json` (kept identical by
  `tools/check_versions.py`, enforced in CI and the release gate).
  A release tag `vX.Y.Z` must match the app version.
- Addon: `19.0.x.y.z` (Odoo series prefix + own sequence).
- Print cores (`print-core`, `print-render`, `print-raw`,
  `print-win`) are internal crates, versioned with the workspace.

## Upgrade policy

- Installers upgrade in place; the local queue database migrates
  in place (additive columns only) and unfinished jobs survive an
  upgrade. Terminal jobs are never resurrected by an upgrade.
- The addon upgrades through Odoo's normal module upgrade; job-log
  history is preserved (schema changes ship with migrations).
- Downgrades are not supported: the queue database and the job log
  only migrate forward.

## Deprecation

A bridge method or endpoint slated for removal is first marked
deprecated in `docs/api-contract.md` for one full minor release of
both sides, then removed in the next major API version (v2), never
inside v1.

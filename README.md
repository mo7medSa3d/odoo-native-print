# Odoo Native Print Client

A native app (Tauri 2, Rust core) that shows a company's Odoo inside an
embedded web view and prints locally on the device. No gateway, no local
background agent, no localhost service, no cloud in the print path.
Platforms in order: Windows, then Android, then iOS.

## Layout
- `crates/print-core` — queue (SQLite/WAL), job state machine, page model,
  paper matching ladder, printer profiles data model, IPC security validation.
  Platform-independent, fully tested. Build/test anywhere: `cargo test`.
- `crates/print-raw` — ESC/POS and ZPL encoders, 1-bit bitmap model, TCP 9100
  transport shape (mock-tested).
- `app/` — Tauri shell (Windows build only, see app/README.md).
- `odoo/native_print` — Odoo 19 addon: profiles, rules, job log, and the
  `ir.actions.report handlers` JS handler (inert in a normal browser).
- `docs/` — research notes, ADRs, threat model, spike results, hardware kit.
- `PROGRESS.md` — current status, risks, and what is blocked and why.

## Status
See PROGRESS.md and docs/spikes/S1-S8.md. Short version: the
platform-independent core is built and green (tests + clippy -D warnings);
everything Windows/Odoo/hardware-bound is explicitly marked BLOCKED with an
exact procedure, not claimed.
# odoo-native-print
# odoo-native-print

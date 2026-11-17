# Native Print — Release Verification & Cleanup Report

**Verification date:** 2026-11-15  
**Source archive:** `odoo-native-print-release-verified.zip`  
**Updated addon version:** `19.0.2.23.0`  
**Desktop app version:** `1.5.7`

## Release disposition

**Not production-certified in this environment.** The available source-level, Python, and Node checks are green, but the Rust workspace could not be formatted, linted, compiled, or tested because `rustc`/`cargo` are absent. Odoo integration tests, Windows MSVC/Win32 printing, and physical-printer verification are also blocked. The archive is a hardened source candidate for CI/build verification, not evidence of a completed production release.

## Root causes addressed

1. **Tauri v2 capability scope was broader than its labels implied.** `windows: ["main"]` matched every WebView in the multi-WebView window, while `remote.json` implicitly retained `local: true`. Capabilities are now scoped by WebView labels only: `main.json` is local-only on `tab-*`, `remote.json` is `local: false` on remote `tab-*`, and `toolbar.json` is local-only on `tabbar`. The remote capability is constrained to the print-PDF, print-status, and app-shell permission groups; queue administration remains local. The command-surface checker now scans all 32 exported command declarations and requires `check_origin()` or `check_local_only()` on every command.

   **Correction to the preliminary audit note:** the previous `main.json` had `local: true`; therefore the source did not establish that its admin permissions were granted to remote Odoo pages. The demonstrated issue was window-wide over-scoping to other WebViews and the remote capability's default local scope, not a proven remote queue-admin leak.

2. **Embedded browser print bypass.** The Tauri bridge now overrides `window.print()` for every top-level remote Odoo WebView, not only receipt pages, so report/receipt paths cannot open the browser's native print dialog and bypass the native queue.

3. **POS retry state could become stale.** The POS watcher now updates `order._printed` on each distinct confirmed `(state, attempts)` outcome. A later successful automatic retry can restore `_printed = true` after an earlier retryable failure set it false. Per-cashier idempotency and `inFlightPosKeys` protection remain in place.

4. **Virtual-printer classification depended on possibly inconsistent metadata.** `kind === "virtual"` now takes precedence over conflicting stale `protocol` or `printer_class` fields in the Odoo client and Control Center. Parity regression cases cover virtual PDF/XPS writers that incorrectly advertise `escpos` or `zpl`.

5. **Duplex reprint settings collapsed “unspecified” into simplex.** A `duplex_configured` marker now distinguishes an explicit `false` (simplex) from no choice. The report handler sends `null` when duplex was not explicitly selected; `action_reprint` omits the duplex prefill unless the original job recorded a deliberate choice.

6. **Concurrent status updates could race across state validation.** The Odoo status controller now row-locks an existing job, refreshes the ORM cache, and re-applies user, terminal-state, transition, and attempts checks after a create-race conflict. Existing company/user scoping and unique-key savepoint handling remain in force.

7. **PDFium peak memory was under-estimated.** The renderer's 128 MiB per-page ceiling now budgets the PDFium bitmap and the simultaneously resident owned pixel copy, and checks the actual aligned row stride before allocating the destination buffer. A planner regression test was added. **This Rust change has not been compiled or executed here.**

## Source-level integration review

- The request structs use camelCase serde mappings for bridge payloads; response DTOs deliberately retain their declared casing, including snake_case nested printer/capability/tab fields. `BRIDGE_JS`, local HTML surfaces, and the Odoo bridge call sites were inspected for their corresponding keys and command names.
- Static Odoo 19 POS integration review covered the payment validation entry point, `ReceiptScreen` manual print, `TicketScreen` past-order print, `pos.printReceipt`, and explicit-element `pos.printer.printHtml` capture. The matching Odoo 19 upstream component files were inspected. This does not replace execution inside a live Odoo 19 server.
- Report handler review covered `qweb-pdf`, `qweb-text`, wizard close handling, reprint-prefill propagation, and `watchJobToCompletion` status metadata. Odoo's public-method access model was considered when hardening `action_reprint`; report and record read access are checked and report group restrictions include implied groups.
- Printer classification was compared across Rust, the Odoo client, and Control Center HTML. The regression test checks the shared protocol/class tokens and virtual-printer precedence.
- Static Win32 source review covered `DeviceCapabilitiesW`, the three-stage `DocumentPropertiesW`/`DEVMODEW` pattern, forms/roll/custom media, tray, orientation, tri-state duplex, color, RAW spooler submission, and GDI failure ambiguity. `pre_write` errors are distinguished from errors after payload submission; only the former are eligible for safe fallback. Ambiguous outcomes are not auto-retried.
- Static TCP 9100 review covered connect-vs-write errors, ESC/POS `DLE EOT` status frames, and 512-dot raster slicing. Static IPP review covered RFC 8011 request attributes, printer URI construction including IPv6 brackets, interim 1xx handling, chunked responses, and monochrome capability. Static rendering review covered DPI selection, page/payload/image limits, PDFium locking/loading, and PNG dimension/pixel limits.

## Executed checks

| Check | Result | Evidence |
|---|---|---|
| `python3 tools/check_command_surface.py` | **PASS** | `command surface consistent: 32 commands guarded, remote reaches 13, admin stays local` |
| `python3 tools/check_versions.py` | **PASS** | Addon `19.0.2.23.0`; app `1.5.7` consistent |
| `node odoo/native_print/tests/test_live_target.mjs` | **PASS** | `JS LIVE TARGET TESTS: ALL PASS` |
| `node odoo/native_print/tests/test_pos_keys.mjs` | **PASS** | `pos key tests: ALL PASS` |
| `node app/tests/control-center.test.mjs` | **PASS** | 9 tests, 9 pass, 0 fail |
| `node app/tests/jsdom/extract-bridge.mjs` | **PASS** | Exit code 0 |
| `python3 -m compileall -q odoo/native_print` | **PASS** | Exit code 0; generated bytecode removed from release ZIP |
| XML well-formedness | **PASS** | 6/6 addon XML files parsed |
| `ar.po` source coverage | **PASS** | 133 non-header entries; 61 `_t/_` literal call sites (57 unique IDs); 0 missing translations, 0 empty Arabic strings, 0 missing module comments, 0 missing source-file occurrence comments |
| `node --check` for `native_print_handler.js`, `pos_native_print.js`, `native_print_client.js`, `native_print_systray.js` | **PASS** | All four syntax checks exited 0 |

## Blocked checks — not represented as passes

- `cargo fmt --all --check` — **BLOCKED:** `cargo: command not found` (exit 127).
- `cargo clippy --workspace --all-targets -- -D warnings` — **BLOCKED:** `cargo: command not found` (exit 127).
- `cargo test --workspace` — **BLOCKED:** `cargo: command not found` (exit 127).
- Five JSDOM suites (`cc-capabilities`, `control-center-behavior`, `errortap`, `navpill`, `watchdog`) — **BLOCKED:** package `jsdom` is absent. `npm ci --offline` could not populate it because `xmlchars-2.2.0.tgz` was not in the local cache (`ENOTCACHED`). The partial install directory was removed from the deliverable.
- Odoo test suite — **BLOCKED:** `odoo`/`odoo-bin`, PostgreSQL tools, and a live Odoo 19 database/server are unavailable.
- Windows MSVC build and Win32 spooler/GDI tests — **BLOCKED:** Windows SDK/MSVC and Windows runtime are unavailable.
- Physical ESC/POS, ZPL, TSPL, EPL, IPP and office-printer tests — **BLOCKED:** no physical devices or live print endpoints are attached.
- `msgfmt` validation — **BLOCKED:** `msgfmt` is not installed. PO syntax and source coverage were checked with Babel's PO parser instead.

## Cleanup scope and remaining risk

Generated `__pycache__`/`.pyc` files and the incomplete `jsdom` install tree were removed. No project source file was deleted based on an unverified “unused” guess. A complete unused-import/dead-code verdict is **BLOCKED** until Rust `clippy` and the project’s full frontend/static-analysis toolchain can run. The repository currently ships `app/dist/tabbar.html` (the tab-bar WebView); there is no separate `toolbar.html` file in this archive.

**Required before production:** run the Rust format/clippy/workspace tests and Tauri build on the supported Windows toolchain; run JSDOM suites with installed dependencies; install the module on a live Odoo 19 + PostgreSQL instance and run the addon test suite; then validate one physical printer per supported transport and at least one GDI-only POS printer/virtual writer. No physical exactly-once guarantee is claimed.

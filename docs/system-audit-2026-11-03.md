# Whole-System Audit — 2026-11-03 (app v1.3.0 / addon 19.0.2.3.0)

A cross-surface consistency audit of the whole product (shell,
bridge, addon, cores, docs), plus an external best-practice
cross-check against official documentation and established products.
Method: mechanical extraction and set comparison wherever two
surfaces must agree, then a full fresh test battery. Findings were
fixed in the same pass unless marked as deliberate design.

## Cross-surface checks (all mechanical, all reproducible)

| Surfaces compared | Result |
| --- | --- |
| Bridge methods called (addon JS + Control Center) vs methods the injected bridge exposes | 19 exposed; every call resolves; every exposed method has a caller (incl. internal bridge callers) |
| Tauri commands: lib.rs ↔ build.rs ↔ permissions ↔ capabilities | Consistent — gated by `tools/check_command_surface.py` (19 commands, remote 13, admin local-only) |
| Job states: print-core `state.rs` ↔ controller `JOB_STATES` ↔ model selection ↔ JS terminal sets (handler + POS) ↔ server state machine | Identical 8 states, identical 4 terminal states everywhere |
| HTTP endpoints: controller routes ↔ JS rpc call sites ↔ api-contract | Exactly 2 (`job_status`, `overview`); no orphan routes, no missing routes; removed routes proven 404 by test |
| Addon manifest: data files + asset bundles vs disk | All present; POS file correctly in `point_of_sale._assets_pos`, backend files in `web.assets_backend` |
| Control Center: `getElementById` ids vs HTML; `onclick` handlers vs defined functions | 30/30 ids exist; 18/18 handlers defined |
| Control Center i18n: Arabic dict vs English dict vs `data-i18n` attributes | 61 keys each, exact parity |
| Origin policy: Rust `parse_origin`/`origin_is_allowed` vs the page's `isAllowedOrigin` | Same rules (HTTPS any host; HTTP any IP literal; no credentials/path/query; ports 1–65535). The page's looser IPv6 regex is unreachable for malformed input because the URL constructor rejects it first; the Rust validator is authoritative at save time |
| Systray OWL template name vs JS `static template` | Match (`native_print.SystrayItem`) |
| Production Rust panic surface | **Zero `unwrap()` in non-test code** across print-core, print-render and the app commands; zero TODO/FIXME/HACK markers |
| Versions: tauri.conf ↔ Cargo.toml ↔ package.json; addon manifest | 1.3.0 everywhere (gated); addon 19.0.2.3.0 |

## Fixed during this audit

1. Control Center version badge fell back to a stale hard-coded
   `1.1.1` when the overview lacked a version — now shows the real
   version or an em dash.
2. The `canceled` job badge rendered lowercase while every other
   badge is uppercase — normalized.
3. Queue durability: `synchronous` raised from NORMAL to FULL.
   With WAL + NORMAL, an OS crash/power loss can roll back the
   last commits — losing an acknowledged job or resurrecting a
   printed one. At human enqueue rates FULL costs nothing
   measurable (bench still passes 1000/1000, no duplicates).
4. ESC/POS band height lowered 1024 → 512 dots. A 1024-row band at
   576 dots is ~74 KB in a single GS v 0 command; small/clone
   thermal printers have small receive buffers and garble or freeze
   on oversized bands. 512 halves the worst case at no measurable
   speed cost; banding correctness is covered by tests.
5. Duplicate-with-different-payload is now logged: a repeated
   idempotency key whose document size differs from the stored job
   records a `duplicate_conflict` event (the first writer still
   wins; the stored job is never overwritten).

## External cross-check (official docs + established products)

Researched against Tauri 2, Microsoft WebView2, Odoo 19, SQLite and
ESC/POS documentation and the architectures of QZ Tray, OCA
base_report_to_printer and Ventor/PrintNode:

- **Tauri ACL model: the recommended pattern.** Remote access is
  opt-in per capability; per-command origin checks in Rust are
  endorsed defence-in-depth, not redundancy. Known pitfall —
  commands missing from the build.rs manifest fail only inside the
  desktop app — is exactly what the command-surface gate prevents.
  Our remote capability uses wildcard URL patterns because the Odoo
  origin is administrator-configured at runtime (it cannot be a
  build-time constant); the binding control is the exact-origin
  check enforced in Rust on every command. Standing duty: keep the
  Tauri dependency current (the ACL had iframe/origin-confusion
  CVEs in the 2.x line; CI's cargo-deny advisories watch this).
- **WebView2: Evergreen + embedded bootstrapper is Microsoft's
  recommended mode** for line-of-business apps. Its one hard caveat
  (an offline machine with no current runtime cannot install) is
  documented in the runbooks and covered by installer matrix case
  I4 in `docs/hardware-test-matrix.md`.
- **Odoo: the report-handlers registry is the core-supported
  interception point**, and POS receipts are a genuinely separate
  subsystem (frontend-generated), so the separate POS path is
  faithful to Odoo's own architecture. Server-side printer storage
  (OCA/Ventor style) was deliberately rejected by the owner in
  favour of QZ-style live discovery; printing therefore exists only
  inside the desktop app — that is the product's locked premise
  (no local agent, no cloud relay), not an oversight.
- **SQLite queue: canonical.** WAL, busy_timeout on every
  connection, a UNIQUE idempotency key, and IMMEDIATE transactions
  for check-then-insert are the documented pattern (a deferred
  transaction's lock upgrade fails with SQLITE_BUSY in ways
  busy_timeout cannot rescue — the exact failure the concurrency
  test caught and the IMMEDIATE change fixed).
- **ESC/POS: GS v 0 with banding is the standard approach**
  (python-escpos fragments the same way). Epson marks GS v 0
  legacy/obsolete in favour of GS ( L; the encoder is isolated in
  print-raw, so a future GS ( L backend is a contained change if a
  target printer ever requires it.
- **Idempotency: matches the accepted model** (client-minted key
  per logical print, first result replayed, deliberate reprints
  mint new keys, ambiguous outcomes never auto-retried, crash
  during printing resolves to a user decision — RECOVERY_AMBIGUOUS
  — never a silent requeue).

## Fresh battery (this audit, this tree)

- Rust workspace: 101 tests pass; print-win: 8; print-core alone
  now 50 (new duplicate-conflict test). Clippy `-D warnings` and
  fmt clean (run in the hardening pass, unchanged surfaces since
  except the audited files, re-run for print-core/print-raw/
  print-render: green).
- Bench 1000/1000 no loss/no duplicates; chaos 300/300 across
  11 restarts.
- Odoo scoped suite: 8/8 on a fresh database.
- Node: live-target, POS keys, Control Center 9/9.
- Bridge jsdom suite: all scenarios pass (watchdog panel, HTTP
  strip + dismiss, healthy/login pages untouched, probes).
- Gates: command surface consistent; versions consistent.

## Standing gates (unchanged, honestly open)

Windows compile + installers are proven by GitHub CI on push;
clean-machine installer cases and the printer matrix run on the
printer-lab hardware (`docs/hardware-test-matrix.md`); code signing
awaits the certificate purchase (workflow already wired). The
server-side addon upgrade on the owner's server and the field
spikes S1–S8 remain owner-side steps.

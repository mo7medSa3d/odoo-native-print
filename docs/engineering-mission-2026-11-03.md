# Principal Engineering Mission — Report (2026-11-03)

Scope: the uploaded GitHub `main` tree (source of truth), audited and
fixed end to end: Odoo addon, Odoo JS/POS, Tauri/Rust desktop,
printing pipeline, Control Center, CI.

## 1. Executive summary

`main` contained work that had never passed through the release gates
of the v1.1.x line: the printer-sync endpoint crashed on every call,
all unique constraints were silently absent on Odoo 19, and the
transport policy had forked into an HTTP-only/IP-only variant that
rejected every normal HTTPS deployment. All Critical and High
findings are fixed and covered by tests. The full verification suite
is green in this environment (Rust 98 tests, Odoo 26/26 on a real
Odoo 19 + PostgreSQL 16, PDFium render 4/4, bench/chaos PASS,
Control Center 9/9 + DOM policy check). Remaining production gates
are unchanged and honest: Windows CI on push, signing certificate,
and the field spikes on real printers.

## 2. Critical findings (fixed)

### C1. Printer sync crashed for every user (100% broken)
`controllers/main.py::sync_printers` read `Device` in the payload
normalization loop before the name was bound further down the
function: `UnboundLocalError` on every call. No device ever synced;
profiles, rules, job routing, the systray and POS printing all depend
on this endpoint. It shipped because the controller had zero tests.
Fix: bind the model at function start. Proof: new HttpCase suite
(`tests/test_controllers.py`) exercising the real HTTP endpoints.

### C2. Second sync would corrupt or fail (upsert blind after sweep)
Sync archives the user's previous devices, then upserted with the
default search context: just-archived records are invisible to
`active_test` searches, so the code fell through to `create` for
printers that already existed. Fix: sweep and upsert searches use
`active_test=False`; duplicate snapshot entries are deduplicated;
an empty snapshot no longer wipes the inventory.

### C3. Unique constraints did not exist on Odoo 19
Both `native.print.device` and `native.print.job` declared
`_sql_constraints`, which Odoo 19 ignores (verified: a fresh install
created no unique index; `\d` showed only the primary key). The
design's core guarantees — one row per printer per company, one log
row per idempotency key per company — were therefore not enforced by
the database at all. Fix: `models.Constraint` (Odoo 19 API) on both
models; addon version 19.0.1.3.0 with a **pre-migration** that removes
older duplicate job rows (keeping the newest per key+company) before
the index is created. Proof: unique indexes present in PostgreSQL
after install; `test_duplicate_job_key_is_rejected` passes.

### C4. Transport policy fork (main rejected HTTPS)
`main` had replaced the origin policy everywhere (Rust, JS,
capability, ADML, docs) with "HTTP only, IP literals only, HTTPS
rejected", with a public test IP baked into the Control Center as
the example. That breaks every standard HTTPS Odoo deployment and
sends sessions over plaintext. Unified on the production policy,
with the owner's explicit amendment recorded in ADR-0002
(2026-11-03): **HTTPS for any host; HTTP for any IP literal
(private or public); hostnames over HTTP rejected.** The Control
Center shows a standing "HTTP is unencrypted" warning on HTTP
origins. Enforced identically in `print-core::security`, the app
config validator, the Tauri capability, and the page's own check,
with parity tests on both sides. Also fixed: the remote capability
URL list contained only `http://` patterns, which would have denied
IPC to HTTPS origins entirely.

## 3. High findings (fixed)

- **Fake job outcomes.** `native.print.job` had Mark Done / Retry /
  Cancel buttons that only rewrote the Odoo log (a user could mark
  a failed job "done" without printing). Removed; job state can now
  change only through the controller's status-sync context
  (`write()` guard), and the job user ACL is read-only.
- **Cross-user job tampering.** `job_status` accepted updates for
  any user's job by key. Updates are now owner-scoped (other user's
  job → `forbidden`); `rules`/`overview` got the same Native Print
  group check `sync_printers` already had. Job record rules added:
  users see their own jobs, managers their company's.
- **Idempotency overwrite (desktop queue).** A duplicate submission
  with the same key returned the existing job id but then overwrote
  its payload/options (`INSERT OR REPLACE`) — a queued or in-flight
  job could silently print different bytes, and a "done" job could
  absorb an orphan payload. New `Queue::enqueue_new`: duplicates
  return the original job untouched. Covered by a queue test.
- **Auto-retry after ambiguous submission failures.** Spooler/TCP/
  IPP errors raised *after* bytes may have reached the printer were
  classified retryable, so the worker auto-reprinted — duplicate
  physical output. New `BackendError::ambiguous` class (never
  auto-retried) for SPOOLER_RAW_ERROR, GDI_PRINT_ERROR,
  TCP9100_ERROR and IPP_PRINT_ERROR; pre-submission failures
  (offline, stopped, discovery) stay retryable. Covered by a worker
  test.
- **Unknown print modes degraded silently.** A typo'd mode sent raw
  PDF bytes to port 9100 (TCP path) or silently fell back to GDI
  (spooler path). All dispatch arms are now explicit; unknown modes
  fail loud with `UNKNOWN_MODE`.
- **Client/server routing divergence.** `print-core::resolve_rule`
  pooled exact and catch-all rules by raw priority and kept the
  *last* rule on ties (despite its doc comment). Rewritten to the
  server semantics: exact-report tier outranks catch-all, priority
  within a tier, earliest rule wins ties. Tests added.

## 4. Medium findings (fixed)

- Terminal job states were not final server-side: a late out-of-order
  status post could move a job out of done/failed. Terminal states
  are now monotonic; late posts are acknowledged as ignored.
- `_rule_payload` hardcoded `scale_policy: "custom_allowed"`,
  silently defeating the profile's strict paper policy. The real
  profile value is now sent (test asserts `strict` passes through).
- Profile counters (`rule_count`, `job_count`) counted nothing real;
  replaced with proper One2many relations + depends. A rule whose
  profile belongs to another company is now rejected
  (`@api.constrains`, test included).
- Device `forms_data` JSON could be truncated mid-string by the
  64 KB cap; serialization now truncates at whole-form granularity.
- Report/POS JS: the initial "queued" job-status RPC could reject
  and block the whole print flow — now fire-and-forget; status watch
  windows extended (~5 min reports / ~3 min POS, with backoff)
  instead of giving up after ~18–24 s.
- Remote capability over-privilege: the Odoo origin could invoke
  queue-admin commands (retry/cancel/clear), diagnostics export and
  settings. Permissions split: remote = submit + read-only status;
  queue admin, diagnostics and origin saving are local-only
  (`allow-queue-admin`, local capability only).
- Explicit paper overrides are now verified against driver-reported
  roll forms when available (80 mm request vs 58 mm roll →
  `PAPER_MISMATCH`) instead of bypassing the media check.
- `odoo.yml` ran only on release tags; addon regressions were
  invisible on `main`. It now also runs on push/PR touching
  `odoo/**`.
- Bridge advertised version `1.0.0` while the app is 1.1.1; it now
  reports the real package version.

## 5. Low / accepted (documented, not changed)

- Printer identity is the Windows display name end-to-end (no stable
  device ID). Renaming a queue retargets server profiles by design
  of the current model; flagged for a future release (port+driver
  hash exists server-side in `printer_key`).
- Single-threaded worker: one printer's retry backoff can delay
  others; worker errors are swallowed without logging; startup
  recovery counts are discarded. Reliability-visible, not
  correctness-critical.
- `client_id` is a browser-minted UUID (not attested by the
  desktop); impact is self-scoped because rules/devices are also
  user-scoped.
- Server cannot deduplicate double-submits beyond the browser's key
  (the desktop queue dedupes by key; status posts are idempotent by
  the unique constraint + owner scoping).
- IPC payloads are materialized before the size cap is applied
  (50 MB/job cap is enforced per page afterwards).

## 6. Verification (this environment, 2026-11-03)

| Suite | Result |
|---|---|
| Rust workspace tests (`cargo test --workspace`) | 90 passed, 0 failed |
| print-win stub tests + clippy | 8 passed, clean |
| Workspace clippy `-D warnings`, rustfmt | clean |
| PDFium render tests (`PDFIUM_TEST_REQUIRED=1`) | 4/4 real renders |
| Bench / chaos (print-tools) | PASS 1000/1000, PASS 300/300, no duplicates |
| cargo-deny 0.20.2 | advisories/bans/licenses/sources ok |
| Odoo 19 + PostgreSQL 16, `--test-tags /native_print`, fresh DB | **26 tests, 0 failed, 0 errors**, exit 0 |
| Unique constraints in PostgreSQL | both present (job + device) |
| Control Center tests (`node --test`) | 9/9 |
| Control Center DOM check (jsdom) | loads clean; public-IP HTTP valid, private HTTP valid, HTTPS name valid, HTTP hostname invalid with inline error |
| Config validator tests (standalone crate) | 2/2 |

The earlier full-database run (all modules) showed 3 additional
failures inside Odoo's own base/web suites in this VM; the
native_print suite itself is fully green, which is the scope of
this project.

## 7. Not tested here (hardware / platform gates, unchanged)

- Windows compile of the Tauri app and the GDI backend, MSI/NSIS
  installers: produced by GitHub Windows CI on push (the previous
  main CI was green; this tree must be pushed to re-run it).
- Real printer behaviour: spooler discovery on a real Windows
  machine, GDI output, ESC/POS receipts, label media, POS flow,
  offline/paper-out recovery on physical devices (spikes S1–S8,
  printer-lab workflow).
- Code signing: installers remain unsigned until a certificate is
  provided (release workflow marks them `-unsigned` honestly).

## 8. Remaining production blockers

1. Push this tree to GitHub `main` (the connected app is read-only;
   the owner pushes) → Windows CI + installer artifacts.
2. Signing certificate for distributable installers.
3. Field gates at the client site: S1–S8, 72 h soak, pilot.

Versions: app 1.1.1 (unchanged), addon 19.0.1.3.0.

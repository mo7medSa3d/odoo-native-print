# PROGRESS

Last updated: 2026-11-03 (Africa/Cairo)
Owner spec: Master Build Prompt — Odoo Native Print Client (Windows, then Android, then iOS)
User: Mohamed. Field testing (Windows, Odoo 19, real printers) will be run
at the client's site, per the user's instruction on 2026-11-02.

## Status: v0.4.0 Release Candidate 4 — full GitHub CI: everything builds and tests on GitHub (Linux + Windows + Odoo + printer lab)

### Evidence (2026-11-03, Linux build VM, Rust 1.99.1)
- `cargo test --workspace`: 72 passed, 0 failed (print-core 36, print-ipp 4,
  print-raw 12, print-render 20) + print-win standalone: 3 passed.
- RC3 — POS receipts: research showed Odoo's own IoT path prints the
  receipt as an image (htmlToImg), so the addon now does the same through
  our bridge: a POS asset patches ReceiptScreen.printReceipt, rasterises
  the on-screen receipt to PNG (styles/images/canvases inlined, 2x), and
  sends it to the new app command print_image. Server rules resolve the
  source name `point_of_sale.receipt` (new report_name field on rules).
  Native pipeline (print-render::image, 6 new tests): defensive PNG
  decode, alpha-over-white grayscale, area-average downscale to
  576/384 dots, threshold, ESC/POS raster stream. Fallbacks preserved
  everywhere: no bridge / no rule / basic receipt / any capture error ->
  default POS printing. Field verification: spike S2b.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean. `cargo fmt`: clean.
- Bench: 1,000 jobs at ~9,200 jobs/sec, no loss, no duplicates.
  Chaos: 300 jobs, ~30% transient failures, 11 restarts: no loss, no duplicates.
- Calibration: PDF + thermal page PASS.
- RC2 additions, all with tests: routing engine (rule resolution, printer
  glob patterns, healthy-printer preference), roll-paper ladder, queue
  backpressure (500 active/printer default) + cancel + durable payloads,
  full printer status sensor decoding (jam/toner/door/pause...), TCP 9100
  transport + DLE EOT status reads, TSPL and EPL encoders, a complete IPP
  client (RFC 8010/8011) verified against a mock IPP printer (attributes,
  format choice, Print-Job byte-exact, document-format sent exactly once),
  Windows discovery/status FFI (EnumPrinters/GetPrinter level 2),
  qweb-text kitchen-ticket path end to end (addon handler -> app command
  -> ESC/POS payload in the durable queue).
- Research pass 2 recorded: Windows discovery/status, IPP protocol,
  Odoo POS/formats (incl. the honest POS-receipt scope note).
- Addon: Python compiles, XML parses, JS parses as ES module.
- Windows-only sources (app shell, print-win FFI): rustfmt parse-checked;
  compilation is a Windows CI gate (.github/workflows/ci.yml).

### Built
- print-core: SQLite/WAL queue, idempotency, strict state machine, crash
  recovery (ADR-0004), worker engine with retry/backoff/dead-letter,
  paper ladder (fail-loud), page model, printer model + status decoding,
  versioned profile/quirks DB, origin + input security validation,
  redacted diagnostics bundle.
- print-render: PDF inspection parser (+ fuzz target and fuzz-smoke test),
  render planning under a 128 MiB/page ceiling, mixed-size sub-job
  splitting, grayscale canvas, calibration generator (A4 PDF with rulers,
  100 mm square, Code 39; 80 mm thermal bitmap).
- print-raw: ESC/POS raster (decoder + TCP loopback tests), ZPL ^GFA,
  1-bit bitmap model.
- print-win (Windows-only): GetDefaultPrinter, DeviceCapabilities forms,
  RAW spooler submission FFI.
- app (Tauri 2, Windows build): config-locked exact origin, navigation
  guard, new-window deny, injected bridge, commands validate origin in
  Rust and inspect PDFs before queueing. Tauri floor >= 2.11.1.
- odoo/native_print (Odoo 19): profiles, rules, job log (list/form/search),
  groups + ACLs + multi-company record rules, rules/status JSON-RPC
  controllers, report handler (server-resolved rules; inert in browsers),
  addon tests, Arabic i18n.
- Enterprise: CI (Linux + Windows + audit/deny), deny.toml, ADMX/ADML GPO
  templates, runbooks, Odoo setup guide, API contract v1, threat model,
  ADRs 0001-0004, hardware validation kit, release checklist.

### Remaining gates (run at the client's site — docs/release-checklist.md)
1. Windows CI green; spikes S1, S3, S4, S5, S8 on Windows.
2. Spike S2 on the production-like Odoo 19.
3. print-win GDI backend (PDFium bitmap -> printer DC) proven by S5 —
   the single remaining Windows-only build item.
4. Hardware kit PASS sheets per printer class; signing certificate;
   72h soak; RDS/Citrix check; pilot rollout.
5. Android phase, then iOS phase (spikes S-A1/S-A2 first).

### Risks
- R1: GDI/PDFium edge cases (clipping on some models) — mitigated by the
  paper ladder, per-printer quirks DB, calibration page, and the kit.
- R2: Tauri remote IPC — mitigated by exact-origin capability, version
  floor, in-Rust origin validation, navigation lock (threat model).
- R3: Nothing in the remaining gates can be proven from this Linux VM;
  RC3 must not be described as production-released until they pass.

### RC4 additions (2026-11-03): GitHub is the build/test home
- Four workflows (docs/ci.md): ci.yml (Linux full suite + addon checks +
  bench/chaos evidence artifacts; Windows workspace+print-win tests on
  MSVC; Windows Tauri build producing MSI+NSIS artifacts; hosted-runner
  printer discovery smoke), release.yml (tag -> gates -> installers +
  source tarball + SHA256SUMS -> GitHub Release), odoo.yml (Odoo Docker
  + Postgres: addon install + tests), printer-lab.yml (self-hosted
  Windows runner with real printers: discovery, status sensors, forms,
  optional real ESC/POS calibration + ZPL test prints).
- npwin_probe tool (print-win): list/status/forms/escpos-test/raw —
  the probe CI and the lab run; verified on Linux (exit codes, stub
  errors) and formatted/clippy-clean; real FFI runs on Windows CI.
- App packaging prerequisites completed: package.json + lockfile
  (npm ci verified here, Tauri CLI 2.12.4), dist/ boot placeholder,
  generated app icons (ICO/PNG), version aligned to 0.3.0.
- Found+fixed by the new clippy gate: unused import in print-win
  discovery.rs. All Linux-side workflow steps were executed locally and
  pass: 72 workspace tests + 3 print-win, fmt/clippy clean everywhere,
  bench/chaos/calibration PASS, workflow YAML parses (4 files, 10 jobs).
- Honest limits: workflows run only after the repo is pushed to GitHub
  (no GitHub remote/credentials in this VM); the first Windows run is
  still the first real compile of the Tauri app — expect to iterate
  from its uploaded logs if it complains.

### RC5 additions (2026-11-03): no bundled WebView2, PDFium embedded, Odoo tested live
- WebView2: installers bundle NO WebView2 installer any more
  (`webviewInstallMode: skip` in tauri.conf.json). The app uses the
  system runtime — Windows 11 ships it; Windows 10 needs Edge/WebView2
  present. This removes ~127-190 MB per installer (the RC4 artifact
  mystery: the 440 MB artifact was two installers each carrying the
  full offline WebView2 installer). Requirement is documented in
  docs/runbooks.md (Install step 2) and docs/odoo-setup.md; the
  now-pointless CI step that installed WebView2 on the build runner
  was removed (the app is never launched in CI).
- PDFium is embedded — nothing left to install for PDF printing:
  pinned bblanchon/pdfium-binaries chromium/8125 (PDFium
  158.0.8125.0, non-V8 build). pdfium.dll (7,586,816 bytes, sha256
  4b0dec92...) is vendored at app/src-tauri/resources/pdfium.dll,
  bundled into both installers via bundle.resources, license beside
  it; full provenance in docs/pdfium.md. tools/fetch-pdfium.sh
  (sha256-verified) fetches the Linux build for dev/CI.
- PDF rendering is real now (was: planning parser only):
  print-render::pdfium binds PDFium dynamically via libloading
  (search: NATIVE_PRINT_PDFIUM_PATH, exe dir, exe resources/, system),
  renders through the existing render plan (DPI ladder + 128 MiB/page
  ceiling, enforced on the actual allocation too) into the existing
  bitmap model (BGRA / grayscale / MonoBitmap threshold). Missing
  library = loud error, no fallback. Linux integration tests render
  the calibration A4 against the real library: 4/4 pass, and CI runs
  them with PDFIUM_TEST_REQUIRED=1 so they cannot silently skip.
- GDI backend (the S5 build item) is code-complete: print-win::dib
  (platform-independent DIB construction + page-fit math, unit-tested
  on Linux) and win::print_pdf_gdi (CreateDC/StartDoc/StretchDIBits
  over PDFium bitmaps). npwin_probe gains `pdf-gdi <printer> <file>`
  and `pdf-gdi-test <printer>`; the Printer Lab workflow has a gated
  pdf-gdi-test step. Its first compile is GitHub Windows CI after
  push — it has NOT compiled yet; no Windows result is claimed.
- Odoo 19 was installed IN THIS VM (Postgres 16 + Odoo 19.0 source in
  a venv; Docker was not used — the VM's apt route was the reliable
  one) and the addon tests were run with the odoo.yml command. The
  run caught a real bug the static checks had missed for four RCs:
  the job search view used `<group expand="0" string="...">`, which
  Odoo 19's RNG rejects; fixed to the modern bare `<group>` form.
  Final result (log: ~/workspace/rc5-evidence/odoo-test-green.log):
  "odoo.tests.result: 0 failed, 0 error(s) of 1563 tests",
  Odoo exit code 0 — all 4 native_print tests pass (job log states,
  POS receipt rule by source name, profile defaults, rule priority).
  An earlier run in this VM's default shell env showed 2 failures in
  Odoo core's own TestConfigManager (data_dir /root vs /home/hatch
  from the VM's uid-0-with-foreign-HOME setup, plus proxy env vars
  leaking into the config); with a clean env (HOME=/root, proxy vars
  unset) the same tree is 1563/1563 green, which pins those two on
  the VM shell, not on any code. In GitHub's odoo.yml the Docker
  image provides the clean env (and wkhtmltopdf/cairo) by default.
- odoo.yml now also runs on pushes/PRs to main touching odoo/**.
- Local gates for RC5: fmt (incl. Windows-only sources) clean,
  clippy -D warnings clean (workspace, print-win, and the Tauri app —
  the app was cargo check + clippy verified on Linux with GTK dev
  libs installed; that first-ever app clippy run surfaced 3
  pre-existing lints — unused API-contract fields in commands.rs and
  an io::Error::new(Other) — fixed in RC5 with the contract fields
  documented as informational), 76 workspace tests + 7 print-win tests pass,
  cargo-deny (0.20.2): advisories/bans/licenses/sources ok,
  bench 8,771 jobs/sec PASS, chaos PASS, calibration PASS, addon
  static checks pass, all 4 workflow YAMLs parse.
- App version aligned to 0.5.0 (package.json, tauri.conf.json,
  src-tauri/Cargo.toml); app Cargo.lock is now committed.
- Honest limits / BLOCKED: Windows compile of the GDI backend and of
  the Tauri bundle with the new resource wiring = first GitHub
  Windows CI run after the user pushes (this VM cannot compile or
  run them; the GitHub App here is read-only, so the user pushes).
  Real-printer proof (S1/S3-S8 field items), signing certificate,
  soak/pilot: unchanged, at the client's site. RC5 must not be
  described as production-released until the Windows CI run is green.

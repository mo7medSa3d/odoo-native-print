# Release Checklist — v0.4.0 Release Candidate 4

## Green here (evidence in repo)
- [x] cargo test --workspace: 72 passing + print-win standalone 3 passing
- [x] POS receipts: ReceiptScreen patch + receipt->PNG rasterizer in the
      addon, print_image command, print-render::image pipeline
      (decode/scale/threshold/ESC-POS) — unit-tested end to end
- [x] cargo clippy --workspace --all-targets -- -D warnings: clean
- [x] cargo fmt: clean (including Windows-only sources, parse-checked)
- [x] Bench: queue ~9,200 jobs/sec, 1,000/1,000 no loss, no duplicates
- [x] Chaos: 300 jobs, 11 restarts, ~30% transient failures: no loss, no duplicates
- [x] Calibration PDF + thermal bitmap generated and round-trip verified
- [x] Routing: rules, glob patterns, healthy-printer preference — tested
- [x] Roll paper, backpressure, cancel, durable payloads — tested
- [x] IPP client against a mock IPP printer (attributes, format choice, Print-Job) — tested
- [x] TCP 9100 + DLE EOT status, TSPL, EPL, ESC/POS, ZPL — tested
- [x] Printer status sensor decoding (all PRINTER_STATUS_* states) — tested
- [x] Addon Python/XML/JS syntax-verified; Tauri JSON configs verified
- [x] Threat model, ADRs 0001-0004, API contract v1, runbooks, setup guide
- [x] CI workflow incl. Windows job and cargo audit/deny; deny.toml; ADMX templates

## Gates that run at the client's site (their testing, per plan)
- [ ] Windows CI job green (app + print-win compile) — first Windows run
- [ ] Spikes S1 (real Tauri IPC origin cases), S3, S4 (50 MB timing), S8
- [ ] Spike S2 on the real Odoo 19 (form/list/wizard, browser fallback)
- [ ] Spike S2b POS receipts on the real Odoo 19 (manual + auto print,
      reprint, long receipt with logo + QR; see docs/spikes/S1-S8.md)
- [ ] Spike S5: Microsoft Print to PDF at A4/A5/80mm/100x150, output verified
- [ ] print-win GDI backend completed against the S5 harness (the one
      remaining build item that is Windows-only by construction)
- [ ] Hardware kit: one PASS sheet per printer class (laser, inkjet,
      thermal USB + network, label 203 + 300 dpi, IPP, shared, virtual)
- [ ] Code-signing certificate applied to MSI/EXE; signed update channel
- [ ] 72-hour soak on a pilot terminal; RDS/Citrix session check
- [ ] Pilot: one department, one week, job-log review, then broad rollout

## Known limitations in RC1 (honest)
- PDF inspection uses a built-in parser for planning; PDFium on Windows
  is the authoritative renderer and replaces planning values at print time.
- The app's byte transfer uses standard Tauri invoke serialisation until
  the S4 raw-body optimisation is verified on Windows.
- TSPL/EPL encoders, Bluetooth/USB-OTG, Android and iOS are later phases
  by the agreed platform order (Windows first).

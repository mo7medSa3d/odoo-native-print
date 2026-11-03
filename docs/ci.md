# CI / GitHub Workflows

Everything is built and tested on GitHub. Four workflows live in
`.github/workflows/`. Until the repository is pushed to GitHub they
cannot run — the steps below are written so the first push exercises
all of them.

## ci.yml — on every push and pull request
- **Core (Linux)**: rustfmt (workspace + the Windows-only sources),
  clippy with warnings as errors, the full workspace test suite,
  print-win tests (Linux stub platform), addon static checks (Python
  compiles, all XML parses, both JS assets parse as ES modules), then
  bench / chaos / calibration with the outputs uploaded as the
  `ci-evidence-linux` artifact.
- **Supply chain**: cargo-deny + cargo audit.
- **Windows — workspace tests + print-win**: the whole test suite on a
  real Windows/MSVC runner, clippy, print-win tests (the real winspool
  FFI compiles here for the first time), and the `npwin_probe` tool
  build, uploaded as an artifact.
- **Windows — build the app installers**: Node 20 + Tauri CLI, builds
  the app and its MSI + NSIS bundles, fails if no installer is
  produced, uploads them as `windows-installers-unsigned`. Since RC5
  the bundles embed no WebView2 installer (`webviewInstallMode: skip`;
  target machines are Windows 11, whose runtime is built in) and do
  embed the vendored PDFium DLL as a resource (docs/pdfium.md). The
  Linux job fetches the pinned Linux PDFium first and runs the render
  tests with `PDFIUM_TEST_REQUIRED=1`, so a render regression fails CI
  instead of silently skipping.
- **Windows — printer discovery smoke**: downloads the probe and runs
  `npwin_probe list` on the hosted runner, plus `Get-Printer`. Hosted
  runners normally expose **no real printers** — this job proves the
  discovery code runs on Windows; real printers are covered by the
  Printer Lab below.

## release.yml — on tags `v*` (or manually)
- Linux gate: the full suite must pass on the tagged tree.
- Windows: tests, then the Tauri build; installers are collected with
  a `SHA256SUMS.txt`.
- Publish: creates the GitHub Release (pre-release when the tag
  contains `-rc`) with the MSI, the NSIS exe, the source tarball and
  checksums. Release notes state plainly that installers are unsigned
  until the company's certificate is wired in (see the comment in the
  workflow for where the signing step goes).

## odoo.yml — manual, on release tags, and on odoo/** changes
Runs on `workflow_dispatch`, on `v*` tag pushes, and on pushes/PRs to
`main` that touch `odoo/**`. Pulls an Odoo Docker image (prefers `odoo:19.0`), starts Postgres 16,
installs `native_print` with `--test-enable --stop-after-init`, and
fails on a non-zero Odoo exit or a failed-test summary in the log.
The full log is always uploaded (`odoo-test-log`) — calibrate the
failure grep from the first real log if it ever misfires. This is the
automated part of spike S2; clicking through the UI and the POS
receipt checks (S2/S2b) still happen at the client's site.

## printer-lab.yml — manual only, self-hosted Windows runner
GitHub-hosted runners have no printers, so real-printer testing runs
on the company's own Windows machine:

1. On the Windows test machine (with the real printers installed),
   register a GitHub Actions self-hosted runner for this repository
   (repo Settings → Actions → Runners → New self-hosted runner) and
   add the label **`native-print-lab`** when configuring it.
2. Run the **Printer Lab** workflow from the Actions tab: enter the
   exact Windows printer names; leave "run test prints" off for a
   discovery/status/forms pass, or on to also print the ESC/POS
   calibration page on the thermal printer and a ZPL test label on
   the label printer.
3. Results land in the `printer-lab-results` artifact — attach them
   to the hardware validation kit sheets (docs/hardware/).

## Versions
- The app version lives in `app/package.json`,
  `app/src-tauri/tauri.conf.json`, and `app/src-tauri/Cargo.toml`
  (all 1.0.0); bump them together with the addon version in
  `odoo/native_print/__manifest__.py` when cutting a release.
- Rust is `stable` in CI; the codebase needs >= 1.75.

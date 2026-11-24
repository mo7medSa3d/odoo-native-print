# CI / GitHub Workflows

Everything is built and tested on GitHub. Four workflows live in
`.github/workflows/`.

## ci.yml — on every push, pull request, and manual dispatch
Top-level `permissions: contents: read` with per-ref concurrency
(`cancel-in-progress: true`). Linux jobs run on `ubuntu-26.04`. Rust is
pinned to **1.90.0** (the floor the pinned Tauri 2.12.5 itself requires;
the workspace declares MSRV 1.89, and the pin makes every job prove the
build at one known toolchain).

- **Core (Linux)**: rustfmt (workspace + the Windows-only sources),
  clippy with warnings as errors, the full workspace test suite,
  print-win clippy + tests (Linux stub platform), pinned PDFium fetch +
  render tests with `PDFIUM_TEST_REQUIRED=1` (a render regression fails
  CI instead of silently skipping), addon static checks for both
  `odoo/native_print` and `odoo20/native_print` (Python compiles, all
  XML parses, every addon JS asset parses), consistency gates
  (`tools/check_command_surface.py`, `tools/check_versions.py`, Control
  Center static tests `app/tests/control-center.test.mjs`, and Odoo 19 & 20 addon node
  tests `test_live_target.mjs` + `test_pos_keys.mjs`), then bench /
  chaos / calibration with outputs uploaded as `ci-evidence-linux`.
  All cargo invocations use `--locked`.
- **Supply chain**: `EmbarkStudios/cargo-deny-action@v2` (using root
  `deny.toml`) and `rustsec/audit-check@v2` against the root workspace
  `Cargo.lock`. Extending `cargo-deny` / `cargo-audit` to
  `crates/print-win` and `app/src-tauri` is not yet wired in `ci.yml`
  (the Tauri dependency tree carries MPL/LGPL/BSL licenses and
  unmaintained GTK3 bindings that the root `deny.toml` rejects).
- **Windows — workspace tests + print-win**: the whole test suite on
  a real Windows/MSVC runner, clippy, print-win clippy + tests (the real
  winspool FFI compiles here for the first time), and the
  `npwin_probe` tool build, uploaded as `npwin-probe`.
- **Windows — build the app installers**: the app crate's unit tests
  (`cargo test --locked` in `app/src-tauri`) run first. Then Node 20 +
  Tauri CLI builds the app and its MSI + NSIS bundles, fails if no
  installer is produced, uploads them as `windows-installers-unsigned`.
  The bundles embed the WebView2 **bootstrapper**
  (`webviewInstallMode: embedBootstrapper`): target machines are
  Windows 11 (runtime built in); on a machine without the runtime, the
  bootstrapper downloads it during install, so install needs internet
  in that case. The bundles also embed the vendored PDFium DLL as a
  resource (docs/pdfium.md).
- **Windows — printer discovery smoke**: downloads the probe and runs
  `npwin_probe list` on the hosted runner, plus `Get-Printer`. Hosted
  runners normally expose **no real printers** — this job proves the
  discovery code runs on Windows; real printers are covered by the
  Printer Lab below.

## release.yml — on tags `v*` (or manually)
- **Linux gate** (`ubuntu-26.04`, Rust **1.90.0**): workspace rustfmt,
  clippy, and tests, PDFium-required render tests, bench and chaos runs,
  consistency gates (`tools/check_command_surface.py`,
  `tools/check_versions.py`, and all 5 `node --test` suites:
  `app/tests/control-center.test.mjs`, plus `test_live_target.mjs` and
  `test_pos_keys.mjs` for both `odoo/native_print` and
  `odoo20/native_print`), and builds + uploads the `source-tarball`
  archive.
- **Windows installers**: `needs` the Linux gate; runs workspace +
  `crates/print-win` + `app/src-tauri` tests on real Windows, optionally
  imports `WINDOWS_CERTIFICATE_BASE64` for Authenticode signing, builds
  the MSI + NSIS installers with Node 20 + Tauri CLI, and generates
  `SHA256SUMS.txt` for the installers.
- **Publish**: runs on `v*` tags (`ubuntu-26.04`), creates the GitHub
  Release (pre-release when the tag contains `-rc`) with the MSI, the
  NSIS exe, the installer `SHA256SUMS.txt`, and the source tarball.
  Release notes reflect the actual signing status passed from the Windows
  build job (reporting Authenticode-signed when
  `WINDOWS_CERTIFICATE_BASE64` is configured, or unsigned otherwise).

## odoo.yml — manual, on release tags, and on odoo/** or odoo20/** changes
Runs on `workflow_dispatch`, on `v*` tag pushes, and on pushes/PRs to
`main` that touch `odoo/**`, `odoo20/**`, or `.github/workflows/odoo.yml`
(`ubuntu-26.04`). Uses a `fail-fast: false` matrix across both pinned
images — `odoo:19.0` (mounting `odoo/`) and `odoo:20.0` (mounting
`odoo20/`, no fallback to floating tags) — starts `postgres:16.4`,
installs `native_print` with
`--test-tags /native_print --stop-after-init --without-demo=all`, and
fails on a non-zero Odoo exit, a failed-test summary in the log, or if
the `native_print` test suite did not execute. The full log is always
uploaded per matrix leg (`odoo-test-log-19.0`, `odoo-test-log-20.0`).
This is the automated part of spike S2; clicking through the UI and
the POS receipt checks (S2/S2b) still happen at the client's site.

## printer-lab.yml — manual only, self-hosted Windows runner
GitHub-hosted runners have no printers, so real-printer testing runs
on the company's own Windows machine (Rust **1.90.0**):

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
  `app/package-lock.json`, `app/src-tauri/tauri.conf.json`, and
  `app/src-tauri/Cargo.toml` — `tools/check_versions.py` enforces
  that all four agree. The addon versions in `odoo/native_print/__manifest__.py` (`19.0.x`)
  and `odoo20/native_print/__manifest__.py` (`20.0.x`) follow independent lines.
- Rust in CI is pinned to **1.90.0** (see above); floating `@stable`
  was retired because a new stable release could change clippy/fmt
  verdicts on an unchanged tree.

## Local verification
Run `python3 tools/verify_local.py` to execute the offline CI checks
(versions, command surface, shell/Python/XML/ADMX/TOML/JSON/workflow
YAML/`ar.po` checks, Odoo ESM syntax, and Node tests) in parallel with
deterministic output and per-step timings. Run
`python3 tools/verify_local.py --self-test` to verify the script's
internal PO parser and parallel runner checks.

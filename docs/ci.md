# CI / GitHub Workflows

Everything is built and tested on GitHub. Four workflows live in
`.github/workflows/`.

> **1.4.8 note:** the workflow files in this tree predate the
> deep-debug CI hardening (pinned Rust, `--locked`, PDFium-required
> render tests, app-crate tests, all-lockfile audit, pinned Odoo
> image). The GitHub App used by the maintainers cannot write
> workflow files, so the hardened workflows ship in the release ZIP
> as `WORKFLOW-CHANGES.md` + patched files for a maintainer to
> upload. This document describes the HARDENED state.

## ci.yml — on every push and pull request
Top-level `permissions: contents: read`. Rust is pinned to **1.90.0**
(the floor the pinned Tauri 2.12.5 itself requires; the workspace
declares MSRV 1.82, but the effective floor is set by the lockfiles'
dependencies, and the pin makes every job prove the build at one
known toolchain).

- **Core (Linux)**: rustfmt (workspace + the Windows-only sources),
  clippy with warnings as errors, the full workspace test suite,
  print-win tests (Linux stub platform), addon static checks (Python
  compiles, all XML parses, every addon JS asset parses), the
  consistency gates (`tools/check_command_surface.py`,
  `tools/check_versions.py`, addon node tests, Control Center static
  tests), the pinned PDFium fetch + render tests with
  `PDFIUM_TEST_REQUIRED=1` (a render regression fails CI instead of
  silently skipping), a fuzz-target compile check, the jsdom DOM
  suites (error tap + Control Center behavior as gates; the watchdog
  timing suite runs non-blocking — its scenarios race real timers
  and flake on loaded shared runners), then bench / chaos /
  calibration with outputs uploaded as `ci-evidence-linux`.
  All cargo invocations use `--locked`.
- **Supply chain**: cargo audit against **all three lockfiles**
  (workspace ~50 packages, app ~420, print-win) + cargo-deny on the
  workspace. Extending cargo-deny to the app tree needs a license
  policy decision first: the Tauri dependency tree carries MPL/LGPL/
  BSL licenses the root allow-list rejects.
- **Windows — workspace tests + print-win**: the whole test suite on
  a real Windows/MSVC runner, clippy, print-win tests (the real
  winspool FFI compiles here for the first time), and the
  `npwin_probe` tool build, uploaded as an artifact.
- **Windows — build the app installers**: the app crate's unit tests
  (`cargo test` in `app/src-tauri`) run first — before 1.4.8 they
  never ran in CI at all. Then Node 20 + Tauri CLI builds the app and
  its MSI + NSIS bundles, fails if no installer is produced, uploads
  them as `windows-installers-unsigned`. The bundles embed the
  WebView2 **bootstrapper** (`webviewInstallMode: embedBootstrapper`):
  target machines are Windows 11 (runtime built in); on a machine
  without the runtime, the bootstrapper downloads it during install,
  so install needs internet in that case. The bundles also embed the
  vendored PDFium DLL as a resource (docs/pdfium.md).
- **Windows — printer discovery smoke**: downloads the probe and runs
  `npwin_probe list` on the hosted runner, plus `Get-Printer`. Hosted
  runners normally expose **no real printers** — this job proves the
  discovery code runs on Windows; real printers are covered by the
  Printer Lab below.

## release.yml — on tags `v*` (or manually)
- Linux gate: mirrors branch CI's Linux job (fmt incl. Windows-only
  sources, clippy, tests, print-win, addon static checks,
  PDFium-required render tests, bench/chaos, both consistency gates
  with the version-vs-tag check). A release must never be weaker
  than a branch build.
- Windows installers: `needs` the Linux gate; workspace + print-win +
  app-crate tests on real Windows, then the Tauri build; installers
  are collected with a `SHA256SUMS.txt`.
- Publish: creates the GitHub Release (pre-release when the tag
  contains `-rc`) with the MSI, the NSIS exe, the source tarball and
  checksums. Release notes state plainly that installers are unsigned
  until the company's certificate is wired in (see the comment in
  the workflow for where the signing step goes).

## odoo.yml — manual, on release tags, and on odoo/** changes
Runs on `workflow_dispatch`, on `v*` tag pushes, and on pushes/PRs to
`main` that touch `odoo/**`. Pulls the pinned `odoo:19.0` image (no
fallback to floating tags — the addon targets Odoo 19 APIs), starts
Postgres 16.4, installs `native_print` with
`--test-enable --stop-after-init`, and fails on a non-zero Odoo exit,
a failed-test summary in the log, or a server banner that is not
"Odoo Server 19". The full log is always uploaded (`odoo-test-log`).
This is the automated part of spike S2; clicking through the UI and
the POS receipt checks (S2/S2b) still happen at the client's site.

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
  `app/package-lock.json`, `app/src-tauri/tauri.conf.json`, and
  `app/src-tauri/Cargo.toml` — `tools/check_versions.py` enforces
  that all four agree. The addon version in
  `odoo/native_print/__manifest__.py` is an independent 19.0.x line.
- Rust in CI is pinned to **1.90.0** (see above); floating `@stable`
  was retired in the 1.4.8 hardening because a new stable release
  could change clippy/fmt verdicts on an unchanged tree.

# Runbooks

## Install (per machine, enterprise)
1. Deploy the MSI silently: `msiexec /i Itbaa.msi /qn ODOO_ORIGIN=https://odoo.company.local:8069`
   HTTP is also accepted for IP literals (e.g. `ODOO_ORIGIN=http://192.168.1.20:8069`) per ADR-0002; hostnames require HTTPS (unencrypted HTTP shows a standing warning in the Control Center).
   (The installer writes `config.json` with `odoo_origin`; or set the GPO policy.)
2. WebView2: installers embed the WebView2 bootstrapper
   (`webviewInstallMode: embedBootstrapper`). Windows 11 ships with the
   runtime; on Windows 10 machines without a current runtime, the
   bootstrapper downloads it during install (requires internet at install
   time unless a current runtime is already present). To check manually:
   `HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`.
3. Install the Odoo addon (`odoo19/native_print` for Odoo 19 or `odoo20/native_print` for Odoo 20; see [docs/odoo-setup.md](odoo-setup.md)). No server-side profiles or routing rules are needed (see [docs/zero-config.md](zero-config.md)).
4. First run: sign in to Odoo inside the app, print the calibration page
   for each printer (app includes the generator), file the hardware kit.

## Upgrade
1. Push the new MSI over the old (same product family, upgrade in place).
   The print queue (per-user data dir) and config survive upgrades.
2. Upgrade the Odoo addon by updating the module in Apps.
3. Re-run one calibration print per printer class after a major upgrade.

## Rollback
Per [docs/compatibility.md](compatibility.md), in-place downgrades are not supported because the SQLite queue database and Odoo job log migrate forward only. If a rollback is required, drain active jobs first, uninstall the newer desktop client before installing the previous MSI, and restore the Odoo database snapshot matching the prior addon version.

## Troubleshooting
- App shows a configuration error: config.json missing/invalid or the
  origin is not HTTPS or is still a placeholder. Fix the origin, relaunch.
- Report downloads instead of printing: the user is in a normal browser
  (expected), OR no local printer is connected, OR the addon is not
  installed/upgraded.
- Job failed with PAPER_MISMATCH: the paper ladder found no acceptable
  form. Do not "fix" it by loosening to a wrong size silently: add the
  right form in the Windows printer driver or adjust per-printer settings
  in the Control Center.
- Job in Dead letter: open the job log, read the error code, fix the cause
  (usually printer offline/paper), then retry from dead letter (state
  transition dead_letter -> queued is supported).
- RECOVERY_AMBIGUOUS after a crash: the job may already be printed. Check
  the printer/output before reprinting; this prompt is deliberate
  (ADR-0004) so duplicates never happen silently.
- Windows 11 thermal printer failures:
  - **LAN TCP 9100 connection refused / timed out**: allow outbound TCP
    port `9100` in Windows Defender Firewall (and confirm the printer's
    static IP responds on port 9100 from the workstation).
  - **Star thermal printers (TSP/mC-Print) printing garbled bytes**:
    switch the printer from StarPRNT / Star Line Mode to **ESC/POS
    emulation mode** (via StarPRNT Utility or hardware DIP switches)
    before using the `escpos` path.
  - **RAW passthrough mangled or dropped on USB/spooler queues**: if a
    vendor GDI driver intercepts or re-renders raw ESC/POS/ZPL/TSPL
    bytes, attach a **Generic / Text Only** Windows print queue to the
    same port for RAW mode (keep the vendor driver for GDI PDF printing).
- Diagnostics: export the one-click bundle (versions, printer list,
  last 200 job summaries, redacted) and attach it to the support ticket.

## Printer onboarding (field staff, ~5 minutes)
1. Install the printer driver; print a Windows test page.
2. Select the printer live from the Odoo systray or Control Center (no server-side profile required).
3. Print the calibration page; measure the 100 mm square; check edges.
4. If content is clipped/shifted, adjust the per-printer offset in the Control Center
   and record it on the validation sheet ([docs/hardware/validation-kit.md](hardware/validation-kit.md)) instead of scaling the document.
5. File the hardware kit result sheet for that printer class.

## White page after sign-in (WebView2 environment)

The Odoo frontend is a large JavaScript application. If the WebView2
environment on a PC cannot run it (stale runtime, blocked storage, or
a poisoned browsing profile), the app no longer fails silently:

- A boot watchdog in the injected bridge watches backend pages; if
  the Odoo client does not mount within ~20s, it shows a diagnostic
  panel with the environment facts (JS level, storage, cookies,
  WebSocket, secure context) and three working actions: Reload,
  Clear web data, Control Center.
- Environment probes are reported to Rust on every Odoo page load
  and appear in the Control Center (Diagnostics tab → WebView2
  card, with the runtime version) and in the diagnostics bundle.
- "Clear web data" wipes the app's cache/cookies/storage/service
  workers and reloads the origin — the one-click cure for a poisoned
  profile. The user signs in again afterwards.
- Installers now use the WebView2 embedBootstrapper: a missing or
  outdated runtime is brought current during installation (requires
  internet at install time, or a machine whose runtime is already
  current).

## Diagnostics bundle privacy

The exported diagnostics bundle is metadata-only BY CONSTRUCTION and
safe to share with support: app/WebView2 versions, queue state
counts, and per-job summaries (printer pattern, report name, page
count, byte size, state, attempts, error code, diagnostics id) plus
the structured job-event log (state transitions and their error
codes). It never contains document bytes or payloads, user names or
credentials, or the Odoo session. The environment section holds the
browser user-agent string and feature booleans (storage/cookies/
WebSocket availability) — nothing that identifies a person. Job ids
in the bundle are the idempotency keys the Odoo job log already
stores; the short `np-...` diagnostics id is the value to quote in
support reports.

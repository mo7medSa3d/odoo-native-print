# Hardware & Installer Test Matrix

Field verification that cannot run in CI or in a Linux VM. Each cell
is run on real hardware via the self-hosted **printer-lab** runner
(workflow `printer-lab.yml`, label `native-print-lab`) or manually
with `npwin_probe`, and the result (pass/fail + probe output) is
recorded in the release notes for the version under test.

Status legend: ☐ not run · ☑ pass · ☒ fail (link the issue).

## Installer on a clean machine (per release)

| # | Case | Expected |
| --- | --- | --- |
| I1 | Fresh Windows 11, no WebView2 interaction by hand, install MSI | Installer brings WebView2 current (bootstrapper), app opens to the Control Center |
| I2 | Fresh Windows 10 22H2, install NSIS setup.exe | Same as I1 |
| I3 | Machine with an outdated WebView2 runtime, install | Runtime updated; Odoo login form visible; backend loads (no white page) |
| I4 | Offline machine with a current WebView2 runtime, install | Install succeeds without internet |
| I5 | Upgrade install over the previous version with queued jobs | Jobs survive; queue opens; version badge shows the new version |
| I6 | Uninstall | App and PDFium removed; queue database left untouched in the app data folder |

## Printer matrix (per release; ☐ until the lab runs it)

| Class | Example class of device | Modes | Paper | Cases |
| --- | --- | --- | --- | --- |
| Thermal receipt (USB, driver) | Epson TM / Xprinter / Rongta 80mm | ESC/POS raster via spooler | 80mm roll, 58mm roll | POS receipt renders at the driver-reported width; long receipt bands completely; cut at end |
| Thermal receipt (network) | LAN 9100 printer | ESC/POS via `tcp://` | 80mm roll | Receipt prints; unplugged printer → `TCP9100_ERROR`, no auto-retry duplicate |
| Label (USB, driver) | Zebra ZD / TSC | ZPL / TSPL raw via spooler | 100×150, 50×30 labels | Label prints at exact size; wrong-size document → `PAPER_MISMATCH`, nothing printed |
| Label (network) | LAN label printer | ZPL via `tcp://` | 100×150 | Same as above over the network |
| Office laser (USB/network driver) | Any PCL/PS laser | GDI (PDF via PDFium) | A4, A5, Letter | Report prints at 100%; duplex on/off honoured; colour/mono honoured; tray selection honoured |
| Office inkjet | Any inkjet | GDI | A4 photo/plain | Report prints; printer offline → `PRINTER_OFFLINE` on the job |
| Virtual | Microsoft Print to PDF / XPS | GDI | A4 | Job completes; output file correct (proves the GDI path without hardware) |

## Environment cases

| # | Case | Expected |
| --- | --- | --- |
| E1 | Printer powered off mid-queue | Job fails with a device error code; queue continues with the next job |
| E2 | Paper out mid-job | `PAPER_OUT`; retry succeeds after loading paper |
| E3 | App killed during printing, restarted | `RECOVERY_AMBIGUOUS` (or auto-reprint per policy); never a silent duplicate |
| E4 | 50 receipts submitted at once | All print in order (priority: receipts first); no loss, no duplicates |
| E5 | HTTP origin (IP) in the app | Unencrypted-connection strip appears once per session; printing unaffected |
| E6 | Poisoned web profile (corrupt cache) | Boot watchdog panel offers Clear web data; one click restores sign-in |

## How to run a cell

1. Install the release candidate from the CI `windows-installers`
   artifact.
2. `npwin_probe printers` — record the discovered printers/forms.
3. Run the case from the app against the test Odoo (report + POS).
4. `npwin_probe pdf-gdi` for the GDI/PDF cells; export the app
   diagnostics bundle and attach it to the release notes with the
   cell results.

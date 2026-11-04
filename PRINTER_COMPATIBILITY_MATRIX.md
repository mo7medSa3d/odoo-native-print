# Printer Compatibility Matrix — Yaseir Print

Status key: **Implemented+tested** (code + automated tests in this
repo), **Implemented** (code exists; hardware verification pending),
**Library only** (implemented with tests, not yet wired into a print
path), **Not implemented**. Hardware verification for every row is
governed by `docs/hardware-test-matrix.md`; at the time of writing
**no physical printer test has been run** (no hardware in the build
environment) — nothing in this matrix claims otherwise.

## Transports

| Transport | Submission | Status source | Notes |
|---|---|---|---|
| Windows spooler, GDI | Implemented | Spooler-reported status (see below) | PDF/PNG rendered by PDFium/DIB pipeline; per-job DEVMODE (paper, orientation, duplex, color, tray, copies). |
| Windows spooler, RAW | Implemented | Spooler-reported status | Write loop + abort-on-error (this cycle). Carries ESC/POS / ZPL / TSPL / EPL byte streams. Write-only: no device readback through this path. |
| TCP 9100 (direct) | Implemented | DLE EOT where the device supports it (library only — see ESC/POS) | Timeouts on connect/write/read; send errors are classified *ambiguous*, never auto-reprinted. |
| IPP | Implemented | IPP attributes (see below) | **PDF documents only** (see IPP row). |

## Protocols

### ESC/POS (thermal receipt)
- Raster: GS v 0 banded at 512 rows (round-trip tested, incl. a
  2500-row receipt). Text lines strip control bytes (no command
  injection). Feed + partial cut emitted at stream end; GS V cut and
  drawer-kick commands exist in the builder.
- Printable widths: derived from the driver-reported roll form on the
  spooler path, which **refuses to guess** when no roll is reported
  (`ROLL_MEDIA_NOT_REPORTED`). Dot tables are the standard 203 dpi
  head densities (58 mm → 384, 80/72 mm → 576, 112 mm → 832); they
  are constants documented as 203 dpi values, not universal truths.
  Unknown paper names are refused (`AUTO_MEDIA_UNAVAILABLE`), never
  silently treated as 80 mm. At the app layer (1.4.12) the 203-dpi
  profile width is scaled by the driver-reported render resolution
  (`preferred_raw_render_dpi`: 203 if reported, else the smallest
  reported resolution in 200–300 dpi, else the smallest reported),
  so a 300-dpi head does not print a physically narrow receipt.
- Status: **DLE EOT n=1** (printer status: offline bit 3, error
  bit 6), **n=2** (offline cause: offline bit 3, drawer pin bit 2,
  online-recovery bit 5 — n=2 carries NO cover-open bit), **n=3**
  (error cause: mechanical bit 2, cutter bit 3, unrecoverable bit 5,
  auto-recoverable bit 6), **n=4** (paper near-end bits 2–3,
  paper-end bits 5–6) — all Library only, added 1.4.12 (n=1/n=3)
  and earlier (n=2/n=4). Responses are frame-validated against the
  fixed bits. None is consulted by a print path yet: a status probe
  must never be interleaved with a print stream, and a Windows RAW
  queue is write-only, so wiring needs a separate bounded
  bidirectional probe (documented limitation). GS r and ASB
  (GS a): **Not implemented**.
- Sources: Epson ESC/POS command reference (DLE EOT frame layouts)
  and the Citizen CD-S500 reference guide, via published excerpts;
  hardware validation NOT RUN.

### ZPL II (Zebra label)
- Builder: `^GFA` graphic fields, `^PW`/`^LL`/`^LH`/`^PQ` options —
  Implemented+tested (shape tests). Print width comes from the
  rendered bitmap; if a queue's largest form exceeds the physical
  head, the device truncates/errors (documented limitation).
- Status: `~HS` / SGD `device.host_status` response parser —
  **Library only** (1.4.12, table-tested for both framings):
  string 1 paper-out / pause / label length / formats-in-buffer /
  buffer-full / partial-format, string 2 head-up / ribbon-out /
  label-waiting / labels-remaining. No response at all while in
  MEDIA OUT / RIBBON OUT / HEAD OPEN / HEAD OVER-TEMPERATURE is
  fault evidence, never readiness — the parser returns `None` for
  an empty response and callers must read that as unknown. Not
  wired into any print path (same bidirectional-probe limitation
  as ESC/POS). Live Zebra status in the app is the Windows spooler
  status only.
- Source: Zebra `device.host_status` SGD documentation, via
  published excerpts; hardware validation NOT RUN.

### TSPL / TSPL2 (TSC label)
- Builder: SIZE/GAP/CLS/BITMAP (mode-0 bit inversion)/TEXT/PRINT —
  Implemented+tested. TEXT content is control-byte sanitized. Gap
  is a builder parameter, threaded since 1.4.12 from the job option
  `label_gap_mm` (3 mm default when the job carries none). Black-mark
  (BLINE) media: **Not implemented**.
- Status: `ESC !?` one-byte reply parser — **Library only**
  (1.4.12, table-tested against the Zebra TSPL guide hex table:
  bit 0 head opened, bit 1 paper jam, bit 2 out of paper, bit 3 out
  of ribbon, bit 4 pause, bit 5 printing, bit 6 cover opened
  option/model-dependent, bit 7 other error; 0x00 = ready). Not
  wired into any print path. Live TSC status in the app is the
  Windows spooler status only.
- Source: Zebra TSPL programming guide (ESC !? table), via
  published excerpts; hardware validation NOT RUN.

### EPL2 (legacy Zebra)
- Builder: `q` width, `Q` height+gap, bitmap, PRINT — Implemented
  (shape tests). The gap is a job parameter since 1.4.12
  (`label_gap_mm`, converted to dots at the render DPI; 24 dots is
  the 3 mm default at 203 dpi).

### IPP (RFC 8010/8011)
- Get-Printer-Attributes: name, state, state-reasons (all values
  preserved), formats, media, color, sides, and (this cycle)
  printer-is-accepting-jobs — Implemented+tested against a mock
  printer, including 100-Continue and chunked responses.
- Print-Job: `document-format` emitted exactly once; success range
  0x0000–0x00FF honored; malformed responses rejected by the parser.
- **Formats: PDF only.** Printers that do not advertise
  `application/pdf` are rejected with `NoUsableFormat`. PWG Raster /
  URF / JPEG are **not** produced (no encoder) and are never claimed.
- Blocking interpretation (library level, wired into the app
  dispatch since 1.4.6): stopped state, not-accepting-jobs, or
  reasons media-empty / media-jam / door-open / cover-open /
  marker-supply-empty / offline / input-tray-missing / output-full
  block submission (`PRINTER_STOPPED`, retryable); warnings
  (toner-low, media-low) do not. IPv6 printer URIs are bracketed
  correctly since 1.4.12 (`ipp_uri`).
- "Accepting jobs" is not "physically ready": an idle IPP printer can
  still be out of paper — the reason set above is the guard, and it is
  only as current as the last attribute poll.

## Windows spooler status (all spooler/GDI/RAW devices)

Decoded from `PRINTER_INFO_2.Status` via `GetPrinterW` into the
multi-condition health model (`PrinterHealth`, 1.4.12): every
reported condition is preserved (a printer can be printing AND
low on paper), each observation carries its source, timestamp and
TTL (30 s), and the Control Center shows conditions, source and a
STALE marker past the TTL. The single primary status is derived
from the same observation, so the badge and the conditions can
never disagree. This is still the **driver/spooler's** view and
can be delayed, generic, or stale on some drivers; it is not a
direct physical sensor read.

| Condition | Decoded from | Blocks printing |
|---|---|---|
| Door open | DOOR_OPEN | yes |
| Paper jam | PAPER_JAM | yes |
| Paper out | PAPER_OUT | yes |
| No toner | NO_TONER | yes |
| Offline / not available | OFFLINE / NOT_AVAILABLE | yes |
| User intervention, manual feed, output bin full | USER_INTERVENTION / MANUAL_FEED / OUTPUT_BIN_FULL | yes (this cycle) |
| Paper problem (generic) | PAPER_PROBLEM | yes (as UserIntervention in the health model; the legacy single-status decoder mapped it to Error — both block) |
| Paused | PAUSED | yes |
| Error (generic) | ERROR | yes |
| Printing / busy / processing | PRINTING / PROCESSING / BUSY | no (activity) |
| Toner low | TONER_LOW | no — separate warning flag |
| Ready | flags == 0 | no |

The worker re-reads live health immediately before printing and
blocks on ANY blocking condition (`PrinterHealth.blocks_printing()`,
1.4.12); a failed read keeps the discovery-time status (checked
moments earlier from the same spooler) as the gate rather than
failing the job on a transient read error, and a failed read is
never presented as Ready. UI status comes from a cached discovery
snapshot (≤20 s at the app layer, 5 s at the print-win layer) and
now carries an observation timestamp + TTL, so stale observations
are marked STALE instead of shown as live.

## SNMP / Printer-MIB (RFC 3805)

**Not implemented.** Network office printers report status via their
driver (spooler) or IPP attributes. SNMP would add supply-level detail
for non-IPP devices; it is deferred deliberately (no default community
strings, bounded probing, and credential handling need their own
design).

## Media and paper

- Windows forms come from `DeviceCapabilitiesW` (DC_PAPERS /
  DC_PAPERSIZE / DC_PAPERNAMES, tenths of a millimetre, count-clamped
  on every array). The paper ladder is deterministic: exact
  (rotation-aware, ±1 mm) → roll-width fit → policy-gated custom size
  inside driver-reported extents → nearest **larger** form; otherwise
  a descriptive paper error. It never returns a smaller form and never
  silently crops or stretches.
- Labels: TSPL SIZE derives from the rendered bitmap at the device
  render DPI (1.4.12: the fit box and the render share one
  driver-reported resolution instead of a hardcoded 203 dpi);
  barcode
  content is rendered (never resampled by a non-integer factor in the
  raster path beyond the documented label-fit downscale, which uses
  nearest-neighbour with no module-width guard — documented
  limitation).
- Arabic/multilingual: PDF reports render through PDFium (embedded
  fonts shape correctly); raw text modes send pre-shaped content —
  arbitrary UTF-8 is never blasted at an assumed ESC/POS code page.

## Device families — tested vs untested

| Family | Path | Automated tests | Hardware |
|---|---|---|---|
| Thermal 80/58 mm (USB, driver) | Spooler RAW/GDI | Encoders, banding, width rules | NOT RUN |
| Thermal network (TCP 9100) | Direct TCP | Mock-printer send, DLE EOT n=1–n=4 decoders (library only) | NOT RUN |
| Zebra label (USB/network) | RAW spooler / TCP | ZPL builder shape + `~HS`/SGD parser (library only) | NOT RUN |
| TSC label (USB/network) | RAW spooler / TCP | TSPL builder shape + sanitization + `ESC !?` parser (library only) | NOT RUN |
| Office laser/inkjet (USB/network) | GDI / IPP | Paper ladder, DEVMODE patterns (compile-gated) | NOT RUN |
| IPP Everywhere printers | IPP | Mock-printer protocol suite | NOT RUN |
| Virtual (PDF/XPS) queues | GDI regression | Classification tests | NOT RUN |

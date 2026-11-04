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
  densities (58 mm → 384, 80/72 mm → 576, 112 mm → 832); they are
  constants documented as 203 dpi values, not universal truths.
- Known gap: one legacy fallback on the direct-TCP path maps unknown
  paper names to 80 mm when no roll metadata was synced (documented
  in the audit; spooler path unaffected).
- Status: **DLE EOT n=2** (offline cause: offline bit 3, drawer pin
  bit 2, online-recovery bit 5) — Library only. **DLE EOT n=4**
  (paper near-end bits 2–3, paper-end bits 5–6) — Library only, added
  this cycle. Responses are frame-validated. Neither is consulted by
  a print path yet. GS r and ASB (GS a): **Not implemented**.
- Source: Epson ESC/POS command reference (DLE EOT frame layouts).

### ZPL II (Zebra label)
- Builder: `^GFA` graphic fields, `^PW`/`^LL`/`^LH`/`^PQ` options —
  Implemented+tested (shape tests). Print width comes from the
  rendered bitmap; if a queue's largest form exceeds the physical
  head, the device truncates/errors (documented limitation).
- Status: `~HS` and SGD `getvar` — **Not implemented**. Zebra status
  in the app is the Windows spooler status only.

### TSPL / TSPL2 (TSC label)
- Builder: SIZE/GAP/CLS/BITMAP (mode-0 bit inversion)/TEXT/PRINT —
  Implemented+tested. TEXT content is control-byte sanitized (this
  cycle). Gap is a builder parameter; the app dispatch currently
  passes a 3 mm default. Black-mark (BLINE) media: **Not
  implemented**.
- Status: `ESC !?` polling — **Not implemented**. TSC status in the
  app is the Windows spooler status only.

### EPL2 (legacy Zebra)
- Builder: `q` width, `Q` height+gap, bitmap, PRINT — Implemented
  (one shape test). The gap is a constant 24 dots (≈3 mm at 203 dpi);
  it is not yet a job/media parameter (documented limitation).

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
- Blocking interpretation (this cycle, library level): stopped state,
  not-accepting-jobs, or reasons media-empty / media-jam / door-open /
  cover-open / marker-supply-empty / offline / input-tray-missing /
  output-full block submission; warnings (toner-low, media-low) do
  not. Wiring `is_blocked()` into the app dispatch is the documented
  remaining step.
- "Accepting jobs" is not "physically ready": an idle IPP printer can
  still be out of paper — the reason set above is the guard, and it is
  only as current as the last attribute poll.

## Windows spooler status (all spooler/GDI/RAW devices)

Decoded from `PRINTER_INFO_2.Status` via `GetPrinterW` — this is the
**driver/spooler's** view and can be delayed, generic, or stale on
some drivers; it is not a direct physical sensor read.

| Condition | Decoded from | Blocks printing |
|---|---|---|
| Door open | DOOR_OPEN | yes |
| Paper jam | PAPER_JAM | yes |
| Paper out | PAPER_OUT | yes |
| No toner | NO_TONER | yes |
| Offline / not available | OFFLINE / NOT_AVAILABLE | yes |
| User intervention, manual feed, output bin full | USER_INTERVENTION / MANUAL_FEED / OUTPUT_BIN_FULL | yes (this cycle) |
| Paper problem (generic) | PAPER_PROBLEM | yes (as Error) |
| Paused | PAUSED | yes |
| Error (generic) | ERROR | yes |
| Printing / busy / processing | PRINTING / PROCESSING / BUSY | no (activity) |
| Toner low | TONER_LOW | no — separate warning flag |
| Ready | flags == 0 | no |

The worker re-reads status live immediately before printing (a failed
read is currently non-blocking — documented in the audit). UI status
comes from a cached discovery snapshot (≤20 s at the app layer, 5 s at
the print-win layer) and carries no observation timestamp yet, so the
UI cannot display status age (documented limitation).

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
- Labels: TSPL SIZE derives from the job's label dimensions; barcode
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
| Thermal network (TCP 9100) | Direct TCP | Mock-printer send, DLE EOT decoders | NOT RUN |
| Zebra label (USB/network) | RAW spooler / TCP | ZPL builder shape | NOT RUN |
| TSC label (USB/network) | RAW spooler / TCP | TSPL builder shape + sanitization | NOT RUN |
| Office laser/inkjet (USB/network) | GDI / IPP | Paper ladder, DEVMODE patterns (compile-gated) | NOT RUN |
| IPP Everywhere printers | IPP | Mock-printer protocol suite | NOT RUN |
| Virtual (PDF/XPS) queues | GDI regression | Classification tests | NOT RUN |

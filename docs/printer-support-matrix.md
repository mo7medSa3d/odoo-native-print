# Printer Support Matrix

Reference matrix of printer classes, protocols, transports, paper-matching policies, and fallbacks implemented in **Itbaa**. See also [compatibility.md](compatibility.md), [error-catalog.md](error-catalog.md), and [adr/0003-paper-ladder-fail-loud.md](adr/0003-paper-ladder-fail-loud.md).

## 1. Supported Printer Classes

| Printer Class | Examples | Default Mode | Typical Media | Notes |
| --- | --- | --- | --- | --- |
| Thermal receipt (58 mm / 80 mm) | Epson TM, Xprinter, Rongta, Bixolon, Star TSP, Citizen, Sewoo, Sunmi, Munbyn, HPRT, generic POS58/POS80 | `escpos` (raster) | 58 mm or 80 mm continuous roll | Always rendered to 1-bit monochrome raster (`GS v 0`) in 256-dot bands (`MAX_RASTER_SLICE_HEIGHT`) to prevent firmware buffer overflow on long receipts. |
| Label printer | Zebra (ZDesigner/ZPL), TSC (TSPL), Xprinter/Gprinter label models (`XP-235`–`XP-490`, `GP-1124`–`GP-3120`), legacy Eltron/Zebra EPL, CPCL mobile/desktop | `zpl`, `tspl`, `epl`, or `cpcl` | 100×150 mm (4×6 in), 50×30 mm, 60×40 mm, custom die-cut | Rendered to 1-bit `MonoBitmap` (hard threshold, no dithering) at driver-reported DPI (203 dpi fallback). |
| Office printer (A4 / A5 / Letter) | HP LaserJet/DeskJet/OfficeJet, Kyocera ECOSYS, Konica Minolta bizhub, Ricoh, Xerox, Brother HL/DCP/MFC | `gdi` (PDFium) or `ipp` | A4 (210×297 mm), A5 (148×210 mm), US Letter (215.9×279.4 mm) | Office exclusion list in [`detect_protocol`](../crates/print-core/src/printer.rs) prevents office queues from being misclassified as thermal POS. |
| Virtual printer | Microsoft Print to PDF, XPS Document Writer, OneNote, Adobe/Foxit/CutePDF/Bullzip/doPDF | `gdi` | A4 / driver-reported forms | First-class silent GDI target; raw printer-language payloads are hard-blocked (`RAW_TO_VIRTUAL_PRINTER`). |

## 2. Protocols

| Protocol | Implementation | Payload Type | Polarity / Framing | Status Polling | Auto-Detection (`detect_protocol`) |
| --- | --- | --- | --- | --- | --- |
| **ESC/POS** | [`crates/print-raw/src/escpos.rs`](../crates/print-raw/src/escpos.rs) | 1-bit raster (`GS v 0`), cut (`GS V`), drawer (`ESC p`), feed | `1` = black dot; 256-dot vertical slices | `DLE EOT` (bidirectional TCP 9100 only) | Yes (`escpos`, class `thermal`) |
| **ZPL II** | [`crates/print-raw/src/zpl.rs`](../crates/print-raw/src/zpl.rs) | `^XA^PW...^LL...^LH...^FO0,0^GFA,...^FS^PQ...^XZ` | `1` = black dot; uppercase ASCII hex | `~HS` / `device.host_status` | Yes (`zpl`, class `label`) |
| **TSPL / TSPL2** | [`crates/print-raw/src/tspl.rs`](../crates/print-raw/src/tspl.rs) | `SIZE`, `GAP`, `CLS`, `BITMAP`, `TEXT`, `PRINT` | `0` = black dot (`!b` inverted from `MonoBitmap`) | `ESC !?` (bidirectional TCP 9100 only) | Yes (`tspl`, class `label`) |
| **EPL2** | [`crates/print-raw/src/epl.rs`](../crates/print-raw/src/epl.rs) | `N`, `q`, `Q<h>,<gap>`, `GW0,0,...`, `P<copies>` | `0` = black dot (`!b` inverted from `MonoBitmap`) | Spooler-reported only | Yes (`epl`, class `label`) |
| **CPCL** | [`crates/print-raw/src/cpcl.rs`](../crates/print-raw/src/cpcl.rs) | `! 0 <dpi> <dpi> <h> <copies>`, `PW`, `CG`, `TEXT`, `FORM`, `PRINT` | `1` = black dot; control bytes stripped in `text_label` | Spooler-reported / TCP connect | Auto-detected via word `cpcl` in `detect_protocol`; full PDF, image, and calibration dispatch |
| **IPP / IPPS** | [`crates/print-ipp/src/client.rs`](../crates/print-ipp/src/client.rs) & Windows Spooler | `application/pdf` only in `print-ipp` (`NoUsableFormat` if unsupported) | RFC 8011 HTTP/1.1 over TCP (port 631); **IPPS** (TLS) via Windows Spooler IPP Class Driver | `Get-Printer-Attributes` (`printer-state`, `printer-state-reasons`, `printer-is-accepting-jobs`) | Network queue / explicit IPP endpoint |
| **Windows GDI / PDFium** | [`crates/print-render/src/pdfium.rs`](../crates/print-render/src/pdfium.rs) & [`crates/print-win/src/win.rs`](../crates/print-win/src/win.rs) | PDF rendered via PDFium to DIB/GDI | Honours `duplex` (tri-state: `None` leaves driver default), `color`, `copies`, `tray` | Win32 `PRINTER_STATUS_*` flags | Default (`gdi`, class `office` / `virtual`) |
| **Raw TCP 9100** | [`crates/print-raw/src/tcp9100.rs`](../crates/print-raw/src/tcp9100.rs) | Raw ESC/POS, ZPL, TSPL, EPL, CPCL bytes | 16 KiB chunked writes with connect + I/O timeouts; close signals end-of-job | Phased error (`Connect` vs `Write`) + protocol status query | `tcp://` or IP port |

## 3. Transports

| Transport | Classification (`PrinterKind`) | Supported Protocols | Live Hardware Health | Notes |
| --- | --- | --- | --- | --- |
| **LAN / IP (Direct TCP 9100)** | `Network` | ESC/POS, ZPL, TSPL, EPL, CPCL | Bidirectional (`DLE EOT`, `~HS`, `ESC !?`) when idle | Connect-phase failure is safe to retry; write-phase failure is treated as ambiguous (no auto-reprint duplicate). |
| **LAN / IP (Direct IPP)** | `Network` | IPP (`application/pdf`) | `Get-Printer-Attributes` blocking reasons (`media-empty`, `media-jam`, `door-open`, `cover-open`, `marker-supply-empty`, `offline`, `input-tray-missing`, `output-full`) | Rejects printers that do not advertise `application/pdf`. |
| **USB / Serial (COM) / Parallel (LPT)** | `Local` | GDI, Spooler RAW (ESC/POS, ZPL, TSPL, EPL, CPCL) | Win32 `PRINTER_STATUS_*` (write-only RAW queues cannot read `DLE EOT`/`ESC !?`) | `stable_printer_id` hashes normalized driver + port (`USB*`, `COM*`, `LPT*`). |
| **Bluetooth** | `Local` (via Windows Spooler) | GDI or Spooler RAW over Windows Bluetooth COM/SPP or vendor driver | Win32 `PRINTER_STATUS_*` | No direct BLE/RFCOMM stack in Rust; must be paired and installed as a Windows printer queue. |
| **Windows Shared Printer (`\\server\printer`)** | `Shared` | GDI or Spooler RAW | Win32 `PRINTER_STATUS_*` from remote spooler | Identified by leading `\\` in queue name or port. |

## 4. Paper Sizes & Scaling Policies

Canonical paper definitions live in [`crates/print-core/src/page.rs`](../crates/print-core/src/page.rs) and matching rules in [`crates/print-core/src/paper.rs`](../crates/print-core/src/paper.rs):

- **Tolerances:** Exact sheet match `±1.0 mm` (`EXACT_TOLERANCE_MM`); thermal roll width match `±2.0 mm` (`ROLL_WIDTH_TOLERANCE_MM`); auto close-sheet fallback `±10%` per side (`CLOSE_SHEET_REL_TOLERANCE`, enabling A4 ↔ US Letter fit).
- **Scaling Policies (`ScalePolicy`):**
  - `strict`: Exact sheet or width-matched roll only; never scales or substitutes; fails loudly (`PaperError`).
  - `custom_allowed`: Exact/roll first, then custom `DMPAPER_USER` size if within driver-reported `min_custom_mm`..`max_custom_mm`.
  - `nearest_larger_centered`: Smallest larger reported form (or wider roll), content centered at 100%.
  - `nearest_larger_fit`: Smallest larger reported form (or wider roll), content scaled to fit.
- **Receipt-to-Office Pagination (`pick_sheet_form`):** A narrow receipt strip (`<= 120 mm`) printed to an office printer paginates onto the office sheet class (A4/Letter, 200–225 mm short edge) rather than failing on height or picking an A5/index-card form.

## 5. Arabic / RTL & Barcode Handling

- **Arabic & RTL text:** Thermal receipt output always uses the **ESC/POS raster path** (`GS v 0`), never printer-resident codepages. Odoo/PDFium renders shaped Arabic ligatures and bidi runs to grayscale, which `MonoBitmap::from_grayscale` converts to 1-bit dots.
- **Barcodes & QR codes:** `MonoBitmap` uses **hard 1-bit thresholding** (no Floyd-Steinberg or ordered dithering) and `PrinterQuirks.force_threshold` enforces monochrome thresholding so quiet zones and bar edges stay crisp at 203/300 dpi.
- **Render DPI selection (`preferred_raw_render_dpi`):** Prefers 203 dpi when reported by the driver; otherwise the lowest reported DPI in 200–300 dpi; falls back to 203 dpi when unreported (flagged `CapabilityProvenance::Unknown`).

## 6. Fallback & Guardrail Behavior

| Condition | Behavior | Error / Action |
| --- | --- | --- |
| Document (`pdf` / `image-png`) sent to Virtual or Office printer with raw mode (`escpos`/`zpl`/`tspl`/`epl`/`cpcl`) | `document_mode_downgrade` automatically re-routes mode to `gdi` | Prints faithfully at the printer's sheet size instead of sending raw bytes. |
| Raw printer-language payload sent to Virtual printer | Hard-blocked by `raw_target_block` | `RAW_TO_VIRTUAL_PRINTER` (prevents corrupt PDF/XPS output). |
| Raw printer-language payload sent to Office printer | Hard-blocked by `raw_target_block` | `RAW_TO_OFFICE_PRINTER` (prevents printing raw command gibberish). |
| Unknown print mode | Rejected before spooling | `UNKNOWN_MODE` (see [compatibility.md](compatibility.md)). |
| Retryable transport/device fault (offline, busy, connect timeout) | Durable SQLite backoff (`1s → 2s → 4s → 8s → 16s → 32s → 60s` cap, up to 10 attempts over ~4 min) | Transitions to `dead_letter` after exhaustion; payload kept for one-tap retry. |
| Ambiguous write/crash mid-print | Never auto-retried (`BackendError::ambiguous` / `RECOVERY_AMBIGUOUS`) | Prevents duplicate receipts/labels; operator decides whether to reprint. |
| No desktop app running (standard browser) | Odoo addon degrades to standard browser download / `window.print()` | No interception; zero breakage. |

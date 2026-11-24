# Print Test & Hardware QA Checklist

Practical pre-release and field verification checklist for `odoo-native-print`. Complements [hardware-test-matrix.md](hardware-test-matrix.md) and [release-checklist.md](release-checklist.md).

Status legend: `[ ]` not run · `[x]` pass · `[!]` fail (attach diagnostics bundle + `npwin_probe` output).

## 1. Per-Paper-Size Verification

### 58 mm Thermal Roll (`escpos`)
- [ ] Driver-reported 58 mm roll matches via `MatchKind::Roll` (`±2.0 mm` tolerance); no horizontal clipping on right edge.
- [ ] Long POS receipt (> 256 dots / > 32 mm tall) prints seamlessly across `GS v 0` bands without missing rows or firmware buffer stalls.
- [ ] Auto-cut (`GS V`) fires cleanly after trailing feed; cash drawer pulse (`ESC p`) opens when configured.

### 80 mm Thermal Roll (`escpos`)
- [ ] Standard 80 mm receipt (576 dots at 203 dpi) renders full width with crisp totals and aligned columns.
- [ ] Narrower 72 mm content fits 80 mm roll without inventing a custom form (`match_paper_auto`).
- [ ] Same receipt printed to an office laser printer downgrades cleanly to `gdi` and paginates onto A4/Letter (`pick_sheet_form`) instead of failing on height.

### 4×6 in / 100×150 mm Label (`zpl`, `tspl`, `epl`, `cpcl`)
- [ ] Exact 100×150 mm label prints at 100% scale with zero drift across a 5-label batch (`copies=5`).
- [ ] Gap sensor aligns cleanly between labels (`GAP` on TSPL, `Q<h>,<gap>` on EPL, `FORM` on CPCL, `^LL` on ZPL).
- [ ] Bit polarity verified: black content prints black on white stock (especially on TSPL/EPL where payload bits are inverted vs ZPL/CPCL).
- [ ] Wrong-size document under `strict` scale policy fails loudly with `PAPER_MISMATCH` and outputs zero blank labels.

### A4 (210×297 mm), A5 (148×210 mm), US Letter (215.9×279.4 mm) (`gdi`, `ipp`)
- [ ] A4 invoice prints at 100% (`MatchKind::Exact`); margins and page breaks match Odoo PDF preview.
- [ ] A5 delivery slip matches A5 form when loaded, or centers/fits on A4 only when `nearest_larger_centered` / `nearest_larger_fit` is enabled.
- [ ] Letter document on an A4-only printer (and A4 on a Letter-only printer) scales to fit cleanly under `match_paper_auto` (`±10%` close-sheet fallback).
- [ ] Tri-state `duplex` verified: `None` leaves driver default untouched; `Some(true)` prints two-sided; `Some(false)` forces simplex.
- [ ] `color` (color vs mono) and `tray` selection honoured on multi-tray office lasers.

## 2. Arabic / RTL & Mixed-Script Rendering
- [ ] Mixed Arabic + English + Latin digits (e.g., VAT invoice header, item names, SKU codes, SAR/EGP totals) render with intact ligatures and correct bidi order on 58 mm, 80 mm, 4×6 label, and A4.
- [ ] No garbled codepage characters on thermal receipts (confirms raster `GS v 0` path is active, not raw text codepage).
- [ ] Arabic UI in Control Center and Odoo Systray (`dir="rtl"`) renders with `Thmanyah Sans`, `letter-spacing: 0`, and non-clipped status pills.

## 3. Barcode & QR Code Sharpness
- [ ] 1D barcodes (Code 128, EAN-13) on 58 mm / 80 mm receipts and 4×6 labels scan on the first pass with a standard handheld laser/2D scanner.
- [ ] ZATCA / invoice QR codes scan reliably with a mobile camera; verify hard 1-bit threshold (no gray dithering speckles inside quiet zones).
- [ ] Smallest bar module stays distinct at 203 dpi (no bleed or horizontal scaling distortion).

## 4. Offline Queueing, Backoff & Idempotency
- [ ] Power off / disconnect printer and submit 3 jobs: all 3 persist in SQLite WAL queue (`queued`) and survive an app restart.
- [ ] Verify exponential backoff (`1s → 2s → 4s → 8s → 16s → 32s → 60s` cap, up to 10 attempts) without blocking jobs queued for a second healthy printer.
- [ ] Reconnect printer before attempt 10: queued jobs drain automatically in priority order (receipts before bulk) with zero duplicates.
- [ ] Re-submit a job with the same `idempotency_key`: deduplicated cleanly, returns existing job ID, never prints twice.
- [ ] Kill app mid-print and restart: interrupted job transitions to `RECOVERY_AMBIGUOUS` (no silent auto-reprint duplicate).

## 5. Multi-Terminal Sharing & Virtual Targets
- [ ] Shared Windows printer (`\\server\printer`, `PrinterKind::Shared`) accepts GDI and RAW jobs from two terminals concurrently without corrupting output.
- [ ] Direct LAN TCP 9100 printer handles sequential jobs from multiple workstations; unreachable host fails at connect phase and retries safely.
- [ ] Direct IPP printer (`application/pdf`) prints A4 PDF; IPP printer without PDF support fails loudly with `NoUsableFormat`.
- [ ] Virtual printer (Microsoft Print to PDF) completes GDI document jobs silently and blocks raw printer-language payloads with `RAW_TO_VIRTUAL_PRINTER`.

## 6. One-Tap Failure Recovery
- [ ] Trigger a device fault (paper out / door open / offline past retry window) so a job lands in `dead_letter` with its error code displayed in Odoo Systray and Control Center.
- [ ] Fix physical fault and click **Retry** once: job immediately clears backoff (`available_at = 0`), transitions `dead_letter -> queued -> printing -> done`, and keeps its monotonic `attempts` counter in the diagnostics log.
- [ ] Click **Cancel** on a queued job: state becomes `canceled` (`CANCELED_BY_USER`) and payload bytes are purged from SQLite.
- [ ] Export diagnostics bundle after a failed test and confirm `diag_id` (`np-<hash>`) correlates with the job event log without leaking raw idempotency keys.

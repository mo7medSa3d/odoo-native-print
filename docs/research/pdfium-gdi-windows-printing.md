# Research: PDFium rendering + Windows GDI printing

Date accessed: 2026-11-02
Sources named in spec:
- https://docs.rs/pdfium-render
- BoxHero engineering post on label printing with Electron/PDFium (precedent)
- Microsoft Learn Win32 printing: DeviceCapabilities, DEVMODE, EnumForms,
  EnumPrinters, GetPrinter, OpenPrinter, StartDoc, spooler notifications

## Summary in my own words (verified via live search 2026-11-02)

1. pdfium-render (docs.rs, current 0.9.x line) exposes page paper size,
   rotation, and render config. `PdfPagePaperSize::Custom(PdfPoints, PdfPoints)`
   exists; sizes are in PDF points (1/72 inch). Our page model will store mm
   internally and convert explicitly (mm -> points -> printer device units)
   in one place, with unit tests.
2. The production pipeline pattern is consistent across precedents:
   render page to bitmap at printer DPI with the PDFium *printing* render
   flags (no LCD/screen smoothing — matters for thermal/barcodes), then move
   the bitmap to the printer DC via GDI (StretchDIBits / BitBlt family), no
   extra scaling, offset by the printer's physical offset.
3. Delphi/VCL PDFium precedent confirms the pitfalls: check the document
   actually loaded before reading page counts (silent zero = silent no-op),
   printing flag changes hinting, and 90/270 rotation needs deliberate
   handling between PDF coordinates and printer coordinates.
4. Windows side (from spec, to be re-verified on a Windows runner):
   DeviceCapabilities for forms/bins/DPI/duplex/color; DEVMODE with
   DMPAPER_USER (dmPaperSize) + dmPaperWidth/dmPaperLength (tenths of mm)
   for custom sizes; verify effective size by querying the created DC;
   EnumPrinters for local/shared/network/virtual; GetJob + change
   notifications for post-spool status.

## Design consequences (implemented in print-core, platform-independent)
- Paper matching ladder with tolerance, exact -> custom DEVMODE -> nearest
  larger per scale policy -> fail loudly. Never silent wrong size.
- Mixed page sizes split into sub-jobs.
- 1-bit threshold rendering mode for thermal/barcodes, no dithering.
- Memory ceiling: render page-by-page, cap DPI by page area.

# Hardware Validation Kit (spec section 12)

Status for every class below: BLOCKED-NEEDS-HARDWARE until a signed result
sheet is filed in this folder. Do not change a status on the basis of a
virtual-printer or mock test.

## How to run a validation (human, ~15 min per printer)
1. Install the Windows build, configure the Odoo origin, add the printer profile.
2. Print the built-in calibration page: rulers on all edges, 100x100 mm box,
   offset marks, Code128 + QR barcode, Arabic sample line, printer/paper/DPI header.
3. Measure the box with a ruler (tolerance: within 1 mm), scan the barcodes
   with a phone, photograph the page.
4. File the result sheet below with the photo attached.

## Classes to cover
- [ ] Laser A4 (USB and network)
- [ ] Inkjet A4
- [ ] 80 mm thermal receipt — USB
- [ ] 80 mm thermal receipt — network (TCP 9100)
- [ ] Label printer 203 dpi
- [ ] Label printer 300 dpi
- [ ] Network printer over IPP
- [ ] Shared printer on a print server (UNC)
- [ ] Virtual printer (Microsoft Print to PDF / XPS) — CI, not hardware

## Result sheet template
Printer model: ______  Driver + version: ______  Connection: ______
Paper (name, measured mm): ______  DPI: ______
Expected: ______  Actual: ______  Box measured: ______ mm
Barcode scans: yes/no  Arabic renders correctly (raster): yes/no
Clipping/offset observed: ______  Profile quirks recorded: ______
Verdict: PASS / FAIL  Tester: ______  Date: ______

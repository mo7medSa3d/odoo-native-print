# Research: Odoo POS, report formats, and what each one needs

First pass: 2026-11-03. POS solution pass: 2026-11-03 (same day, after
the user required POS receipts to be covered, via the image route).

## POS receipts: problem and solution

Facts established by research:
- POS receipts are NOT ir.actions.report reports; the report handler
  cannot intercept them. That part of the original finding stands.
- BUT receipts do not need the report pipeline. Odoo's own printing
  refactor (odoo/odoo, "[REF] pos*: simplify receipt printing", 2023,
  shipped in 17+) removed AbstractReceiptScreen and funnelled receipt
  printing through a POS `printer` service, made the receipt a
  standalone component (`OrderReceipt`) rendered in the page, added a
  `renderer` service for it, and the IoT printer path converts the
  rendered receipt to an IMAGE (htmlToImg) and prints the image.
  Printing the receipt as an image is therefore Odoo's own architecture,
  not a workaround.
- Odoo 19 extension points (verified against Odoo 19 development
  literature): POS addons patch components with
  `patch(Component.prototype, ...)`; ReceiptScreen is imported from
  `@point_of_sale/app/screens/receipt_screen/receipt_screen`; POS
  assets live in the `point_of_sale._assets_pos` bundle. The receipt
  preview lives in the ReceiptScreen DOM (`.pos-receipt` inside
  `.pos-receipt-container`).
- Odoo 19 user docs: without a configured receipt printer, "Print Full
  Receipt" falls back to the browser print function; with automatic
  receipt printing enabled, the receipt prints on payment completion —
  both flows go through the ReceiptScreen's print method.

Implemented solution (RC3), the user's chosen pipeline
(receipt -> image -> native render/adjust -> printer):
1. Addon POS asset `pos_native_print.js` patches
   `ReceiptScreen.prototype.printReceipt`:
   - no bridge, a "basic receipt" request, no server rule, or any
     error -> the original method runs (default POS printing);
   - otherwise it resolves the rule for the source name
     `point_of_sale.receipt` via the existing `/native_print/rules`
     RPC, rasterises the on-screen receipt element to a PNG
     (SVG foreignObject, computed styles + images + canvases inlined,
     2x scale), and calls `bridge.printImage`.
2. Server: `native.print.rule` gained a `report_name` field for
   non-report sources; `/native_print/rules` resolves it (company and
   group filters apply exactly as for reports).
3. App: new `print_image` Tauri command — origin check, full PNG
   decode validation, durable queue with payload kind `image-png`,
   receipt priority.
4. print-render::image: defensive PNG decode (size/dimension caps
   checked before allocation), grayscale with alpha-over-white,
   area-average downscale to the printer's printable width
   (80mm -> 576 dots, 58mm -> 384 dots at 203dpi), threshold 160,
   ESC/POS GS v 0 stream + feed + cut. Unit-tested end to end.

Remaining field verification (spike S2b, client site): on the real
Odoo 19, confirm the ReceiptScreen patch point name and the receipt
DOM classes, print manually and with auto-print, and reprint from the
orders list. If a future Odoo renames the method, the fallback keeps
POS printing working and the patch target is a one-line change.

## Report formats
- qweb-pdf: the report handler's PDF path (unchanged).
- qweb-text: kitchen tickets / simple labels via the text path
  (addon handler branch -> app print_text -> ESC/POS text). Arabic
  never uses text mode; Arabic prints via PDF/raster.
- Attachments: "print this file" fetches /web/content and uses the
  same bridge entry points (PDF via printPdf, PNG via printImage).

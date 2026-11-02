# API Contract v1: Odoo addon <-> Native Print app

Versioned. Breaking changes bump the major version in both sides.

## Bridge (injected by the shell, configured origin only)
`window.__NATIVE_PRINT__.printPdf({ idempotencyKey, reportName, url, bytes })`
- bytes: Uint8Array of the PDF, transferred as a raw body (never base64/JSON).
- Limits enforced in Rust: <= 50 MiB, <= 2000 pages, copies 1..99 (print-core::security).
- Resolution: the addon resolves rule -> profile server-side; the client
  never invents policy from page-supplied values.

`window.__NATIVE_PRINT__.printText({ idempotencyKey, reportName, printerPattern, text, cut })`
- qweb-text reports (kitchen tickets). Encoded to ESC/POS text by the app.

`window.__NATIVE_PRINT__.printImage({ idempotencyKey, reportName, printerPattern, paper, bytes })`
- bytes: Uint8Array of a PNG image. Used for POS receipts (source name
  `point_of_sale.receipt`): the POS extension rasterises the on-screen
  receipt to PNG in the webview and sends the image; the app validates
  it by full decode (print-render::image: <= 20 MiB, dimension caps),
  queues it durably (payload kind `image-png`), and at print time
  scales it to the paper's printable width (80mm -> 576 dots,
  58mm -> 384), thresholds it, and prints it as an ESC/POS raster
  (office profiles print the same bitmap via GDI).

## Status loop
The addon records every job in native.print.job (same state names as the
Rust state machine). The handler reports state changes back by RPC using the
idempotency key; duplicates with the same key are one job, on both sides.

## Open verification items (blocked on Odoo 19, spike S2)
- Exact binary fetch API in the Odoo 19 webclient for /report/pdf/... using
  the user's session (the handler currently uses the http service; if that
  does not return raw bytes in 19, switch to the download/network layer and
  record the change here).
- qweb-html / qweb-text handling and multi-document reports.

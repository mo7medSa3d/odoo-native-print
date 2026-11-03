# Print interception & routing — design decisions

Benchmarked 2026-11-03 against Odoo 19 core, OCA
base_report_to_printer (19.0), VentorTech Direct Print PRO,
PrintNode integrations, QZ Tray apps, and Odoo IoT.

## Interception point

The addon registers a handler in Odoo's official
`ir.actions.report handlers` JS registry — the least fragile point
in the web client (the same mechanism Odoo IoT uses). The browser
fetches the normal `/report/pdf/...` (or text) URL with the user's
session, and the bytes go to the desktop bridge. Rejected
alternatives:

- Overriding `/report/download`: single choke point, but route
  replacement is the most fragile option across versions and it
  cannot fall back gracefully per report.
- Overriding `ir.actions.report._render_qweb_pdf` (OCA's hook):
  catches programmatic renders too (email, EDI), and gives no clean
  "not handled" path back to the download. Reserved for a possible
  future server-triggered auto-print scenario behind an explicit
  context flag — not used today.
- Fields on `ir.actions.report` alone: data without a flow.

POS receipts are a separate flow in Odoo (not report actions); the
addon patches the POS receipt printing path instead, and routes it
through the same rules endpoint with the `point_of_sale.receipt`
source name.

## Routing model (live, per-PC)

Superseded 2026-11-03 by the owner's simplification decision: there
are no server-side routing rules anymore. The target printer is
resolved in the page at click time from the desktop bridge's live
inventory of THIS PC's printers:

1. The printer the user selected in the systray (browser-local
   choice), when still connected.
2. For POS receipts only: the first connected thermal/label
   printer, when the selection is not one.
3. The Windows default printer, else the first printer.

Capabilities and media come from the driver at that same moment
(forms, roll width, duplex, color). One-click selection lives in
the systray printer list ("Use this printer" + star badge). The
server keeps only the job log.

## Failure behaviour (never hard-break printing)

- No bridge (plain browser), unsupported report type, no client id,
  or no matching rule → the handler returns `undefined` and Odoo's
  standard download happens, unchanged.
- Report fetch failure → same fallback; Odoo's own error reporting
  applies.
- Bridge submission failure → the job is logged as failed
  server-side, the user gets a warning toast, and the standard
  download still happens. (Pattern copied from the QZ apps'
  documented "Download PDF instead" fallback.)
- Printer vanished/offline → the rule cannot match an inactive
  device, so routing falls to the next tier or the download.

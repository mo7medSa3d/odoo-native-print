# Zero-config printing — the live model

Owner decision (2026-11-03): printers are NOT stored in Odoo. No
printer records, no profiles, no routing-rule tables, no sharing of
one PC's printers to another PC. The system stays deliberately
simple:

1. The desktop app on the user's own PC enumerates that PC's
   printers (Windows spooler) — that live inventory is the only
   printer truth, and it exists only in the moment.
2. In Odoo, the systray printer menu lists those printers. The user
   can mark one "Use this printer"; the choice is kept only in the
   browser's local storage on that PC. With no choice, the Windows
   default printer is used. POS receipts prefer a thermal/label
   printer when one is connected.
3. At print time the page asks the bridge for the chosen printer's
   real capabilities (driver-reported forms, receipt roll width,
   duplex, color), infers the protocol from the driver identity
   (ESC/POS, ZPL, TSPL, EPL, or GDI through the Windows driver), and
   the desktop fits the output to that printer's real media:
   receipts to the reported roll width, labels downscaled into the
   largest reported label form, office output shrink-only.
4. The only thing Odoo stores is the job log (what printed, on
   which printer name, and the outcome) — an audit trail, not
   configuration.

If no printer is connected at that moment, or anything in the
native path fails, Odoo's standard download happens instead.
Nothing to configure, nothing to go stale, nothing shared between
machines.

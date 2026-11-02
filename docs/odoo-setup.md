# Odoo Setup Guide (addon: native_print)

Target: Odoo 19 (17/18 expected to work; verify on spike S2 environments).

## Install
1. Copy `odoo/native_print` into the server's addons path.
2. Restart Odoo, update the apps list, install **Native Print**.
3. Assign groups: users get "Native Print / User", print administrators
   get "Native Print / Manager".

## Configure
1. **Native Print > Profiles**: one row per printer (or printer family).
   `printer_pattern` must match the Windows printer name on the client.
   Choose the mode: Office (gdi), Thermal ESC/POS, Label ZPL/TSPL, IPP.
   Set paper, copies, and the scale policy. Default recovery mode is the
   safe one (never auto-reprint an ambiguous job).
2. **Native Print > Rules**: link reports to profiles. Higher priority
   wins. Optionally restrict a rule to a user group. Rules are
   per-company; multi-company is enforced by record rules.
3. **POS receipts**: create a profile for the receipt printer (mode
   Thermal ESC/POS, paper `80mm` or `58mm`), then a rule with
   **Source name (non-report)** set to exactly `point_of_sale.receipt`
   (leave the Report field empty) pointing at that profile. POS
   receipts then print natively: the POS screen's receipt is captured
   as an image and printed as a raster on that printer. Without this
   rule, POS keeps its default printing (IoT / browser print).
4. **Native Print > Jobs**: the job log. Filter by state (Failed and
   Dead letter need attention), report, or device.

## Behaviour
- In the Native Print app, a report with a matching rule prints locally
  with no download and no print dialog.
- In any normal browser, or for reports without a rule, Odoo behaves
  exactly as before (normal download). Nothing changes for users who do
  not use the app.
- Arabic and English: the addon ships an Arabic translation file
  (i18n/ar.po); menus and screens follow the user's Odoo language,
  including RTL, because they are standard Odoo views.

## Verify (spike S2 procedure)
1. Open a sale order, Print: the PDF prints on the profile's printer and
   a Jobs row appears (queued -> done) with the right page count.
2. Repeat from a list action and from a wizard.
3. Repeat in a normal browser: a normal PDF download happens instead.
4. POS (spike S2b): with the POS rule configured, complete a sale in the
   app and use **Print Full Receipt**: the receipt prints on the receipt
   printer as an image and a Jobs row with source
   `point_of_sale.receipt` appears. Repeat with **Automatic Receipt
   Printing** enabled in the POS settings, and reprint from the Orders
   list. Repeat in a normal browser: default POS printing happens.
5. Log the results in docs/spikes/S1-S8.md and update its status.

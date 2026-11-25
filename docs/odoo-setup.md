# Odoo Setup Guide (addon: native_print)

Target: **Odoo 19** (`odoo19/native_print`, `19.0.2.24.0`) and **Odoo 20** (`odoo20/native_print`, `20.0.1.0.0`). Older Odoo versions are not supported (see [compatibility.md](compatibility.md)).

## Client machine requirements (the Windows app)

- Windows 11 (any edition), or Windows 10 with Microsoft Edge /
  WebView2 runtime present. The installers embed the WebView2
  bootstrapper (`webviewInstallMode: embedBootstrapper`): Windows 11
  already ships the runtime, and on machines without it the bootstrapper
  downloads it during install.
- PDF rendering needs nothing installed: PDFium is bundled inside the
  installer (see [pdfium.md](pdfium.md)).
- A printer driver installed in Windows for every printer used.

## Install
1. Copy `odoo19/native_print` (for Odoo 19) or `odoo20/native_print` (for Odoo 20) into the server's addons path, or build `Dockerfile.odoo` with `--build-arg ODOO_VERSION=19.0 --build-arg ADDON_SRC=odoo19/native_print` (default) or `--build-arg ODOO_VERSION=20.0 --build-arg ADDON_SRC=odoo20/native_print`.
2. Restart Odoo, update the apps list, install **Yaseir Print** (`native_print`).
3. Assign groups: internal users automatically inherit "Native Print / User";
   print administrators get "Native Print / Manager".

## Configure
1. **Live workstation printer selection**: printers are discovered live
   from each workstation by the desktop app (no server-side profiles or
   routing rules are required). Users can pick their active printer from
   the **Yaseir Print** navbar systray widget (stored in browser storage);
   otherwise the Windows default printer is used.
2. **POS receipts**: inside the desktop app, POS receipts are captured
   as 2x PNG images and automatically routed to a thermal/label printer
   when present (or the selected/default printer otherwise). In a normal
   browser, POS keeps its default printing (IoT / browser print).
3. **Native Print > Jobs**: the per-company operational job log
   (`native.print.job`). Filter by state (Failed and Dead letter need
   attention), report, or device (`native.print.device`, auto-synced
   history/picker registry).

## Behaviour
- In the Yaseir Print desktop app, `qweb-pdf` and `qweb-text` reports
  print locally with no download and no print dialog.
- In any normal browser, Odoo behaves exactly as before (normal
  download). Nothing changes for users who do not use the app.
- Arabic and English: the addon ships an Arabic translation file
  (`i18n/ar.po`); menus and screens follow the user's Odoo language,
  including RTL, because they are standard Odoo views.

## Verify
1. Open a sale order, Print: the PDF prints on the active workstation
   printer and a Jobs row appears (`queued -> done`) with the right page
   count.
2. Repeat from a list action and from a wizard.
3. Repeat in a normal browser: a normal PDF download happens instead.
4. **POS receipts**: complete a sale in the desktop app and use
   **Print Full Receipt**: the receipt prints on the thermal receipt
   printer as an image and a Jobs row with source
   `point_of_sale.receipt` appears. Repeat with **Automatic Receipt
   Printing** enabled in the POS settings, and reprint from the Orders
   list. Repeat in a normal browser: default POS printing happens.
5. Record verification results against [release-checklist.md](release-checklist.md).

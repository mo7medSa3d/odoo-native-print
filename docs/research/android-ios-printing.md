# Research: Android and iOS printing (phase-gated, initial pass)

Date accessed: 2026-11-02. These phases start only after Windows passes its gates.

## Android
Sources from spec: ipp-client-kotlin (gmuth), RFC 8011, PWG 5100.x.
Verified 2026-11-02: Android's OS pipeline is PrintManager, which presents UI;
third-party printing stacks confirm the silent paths are (a) raw TCP 9100
(JetDirect) streaming — with mDNS discovery or subnet sweep for printers that
never advertise — and (b) a self-built IPP client. ESC/POS status readback
uses DLE EOT where the transport allows it.
Plan stands: own IPP client (Get-Printer-Attributes first, send PDF only if
document-format-supported includes it, else PWG/URF/JPEG raster), TCP 9100 for
thermal/label. Spikes S-A1/S-A2 remain to run in the Android phase.

## iOS
Sources from spec: UIPrintInteractionController, UIPrinterPickerController, UIPrinter.
Verified 2026-11-02: since iOS 8, an app can present UIPrinterPickerController
once, save the selected UIPrinter, and later call
UIPrintInteractionController.printToPrinter(_:completionHandler:) to print
without showing UI. Constructing a UIPrinter from a URL is the alternative
path and is known to be unreliable — picker-first is the design.
Plan stands: picker once, save, direct print; TCP 9100 for thermal.
App Store / unlisted / MDM distribution ADR is deferred to the iOS phase.

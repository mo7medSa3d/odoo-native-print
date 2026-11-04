# Error Catalog

Every error the product can surface, in one place. Codes are stable:
they are stored on jobs, shown in the Control Center and the Odoo job
log, and quoted in support reports together with the job's
diagnostics id (`np-...`). Codes never change meaning between
versions; new failures get new codes.

## Queue and lifecycle (print-core)

| Code | Meaning | What to do |
| --- | --- | --- |
| `CANCELED_BY_USER` | The user canceled the job before it printed. | Nothing; informational. |
| `RECOVERED_BEFORE_SPOOL` | App restarted while the job was rendering/spooling; it never reached the printer and was re-queued automatically. | Nothing; it will print. |
| `RECOVERY_AMBIGUOUS` | App restarted while the job was printing; it may already be on paper, so it was NOT reprinted (no duplicates). | Check the printer; reprint manually if nothing came out. |
| `RECOVERED_AUTO_REPRINT` | Same as above, but the auto-reprint policy was on, so the job was re-queued. | Check for a possible duplicate page. |
| `RECONCILED_STALE` | A job sat in a working state far beyond the stale threshold (worker gone or wedged) and was failed by reconciliation. | Retry the job from the Control Center. |
| `MISSING_PAYLOAD` | A queued job's document bytes are missing (interrupted write, disk problem). The job is failed, never printed partially. | Resubmit the document; check disk health if it repeats. |
| `QUEUE_DB_ERROR` | The local queue database reported an error. | Export diagnostics; check disk space and permissions on the app data folder. |

Backpressure is not an error code on a job but a submission refusal:
`QueueFull` (per-printer limit 500 active jobs, global limit 2000)
means the client is submitting faster than the printer works.

## Rendering and documents

| Code | Meaning | What to do |
| --- | --- | --- |
| `PDF_INVALID` | The submitted bytes are not a readable PDF. | Re-export the document from Odoo. |
| `EMPTY_DOCUMENT` | The PDF has zero pages. | Check the report in Odoo. |
| `PDFIUM_LOAD_ERROR` | The bundled PDFium engine failed to load. | Reinstall the app; the engine ships inside the installer. |
| `RENDER_FAILED` | A page could not be rendered for printing. | Export diagnostics with the job's diagnostics id. |
| `IMAGE_DECODE_ERROR` | A receipt/label image could not be decoded. | Retry; if it repeats, export diagnostics. |
| `BITMAP_PACK_ERROR` | An internal bitmap conversion failed. | Export diagnostics. |

Submission-time validation refusals (HTTP-style string errors from
the bridge, before anything is queued): `bad_idempotency_key`,
`Empty`, `TooLarge` (job over 50 MiB), `TooManyPages` (over 2000),
`TooManyCopies`, `BadDimension`, `bad_text_size`.

## Paper and media

| Code | Meaning | What to do |
| --- | --- | --- |
| `PAPER_MISMATCH` | The document's page size does not fit the printer's paper and the scale policy forbids resizing. Nothing was printed (never print on the wrong size silently). | Load the right paper or allow scaling for that report. |
| `PAPER_OUT` | The printer reports no paper. | Load paper; the job can be retried. |
| `AUTO_MEDIA_UNAVAILABLE` | Automatic paper matching found no usable form on the printer. | Check the printer's loaded forms in Windows. |
| `PRINTER_MEDIA_UNAVAILABLE` | The requested form/tray is not available on this printer. | Pick another form or printer. |
| `ROLL_MEDIA_NOT_REPORTED` | A roll printer did not report a roll-width form, so receipt width cannot be verified. | Check the printer driver settings. |
| `ROLL_WIDTH_UNSUPPORTED` | The receipt roll width is not a known print head (58/80/112 mm family). | Use a standard roll. |

## Printer and transport

| Code | Meaning | What to do |
| --- | --- | --- |
| `PRINTER_NOT_FOUND` | No printer matches the selected queue name or address. | Re-select the printer in the app (printers are discovered live). |
| `PRINTER_OFFLINE` | Windows reports the printer offline. | Power/connect the printer. |
| `PRINTER_PAUSED` | The Windows queue is paused. | Resume it in Windows print settings. |
| `PRINTER_STOPPED` | The print spooler service is stopped. | Start the "Print Spooler" service. |
| `PRINTER_BUSY` | The printer is busy with another job. | Wait; the worker retries with backoff. |
| `PAPER_JAM` | The printer reports a paper jam. | Clear the jam. |
| `DOOR_OPEN` | The printer cover is open. | Close it. |
| `NO_TONER` | The printer reports toner/ink empty. | Replace it. |
| `PRINTER_ERROR` | The printer reports a generic device error. | Check the device panel. |
| `PRINTER_CAPABILITIES_UNAVAILABLE` | The driver did not answer the capabilities query. | Update/reinstall the printer driver. |
| `BAD_PRINTER_ADDRESS` | A `tcp://` / `ipp://` printer address is malformed. | Fix the printer URI in the Control Center picker. |
| `SPOOLER_UNAVAILABLE` | The Windows print spooler cannot be reached. | Restart the spooler service. |
| `SPOOLER_RAW_ERROR` | Sending raw bytes (ESC/POS/ZPL/TSPL) through the spooler failed. | Check the queue; export diagnostics if it repeats. |
| `GDI_PRINT_ERROR` | A GDI print job failed on Windows. | Export diagnostics with the diagnostics id. |
| `TCP9100_ERROR` | A direct network print (port 9100) failed. Ambiguous by nature: the job is NOT auto-retried, because the printer may have received it. | Check the printer before reprinting. |
| `IPP_PRINT_ERROR` / `IPP_ATTRIBUTES_ERROR` / `IPP_FORMAT_UNSUPPORTED` | IPP printing failed / the printer rejected the attributes / the document format is unsupported over IPP. | Check the printer's IPP support or use its Windows queue instead. |
| `UNKNOWN_MODE` | A job asked for a print mode the app does not implement. It fails loudly; bytes are never sent in a guessed format. | Update the app/addon pair (see the compatibility policy). |

## Bridge, server and environment

| Code | Meaning | What to do |
| --- | --- | --- |
| `BRIDGE_ERROR` | The Odoo page could not hand the job to the desktop app; the report fell back to Odoo's standard PDF download. | Make sure the desktop app is running. |
| `forbidden` (job_status) | The posting user does not own the job or lacks the Native Print group. | Sign in as the job's owner / check group membership. |
| `bad_request` (job_status) | A status post failed validation (key too long, unknown state). | Update the app/addon pair. |
| `illegal_transition` (job_status, ignored) | A status post tried to move a job backward in the state machine; the recorded state was kept. | Informational; usually a stale watcher. |
| `terminal_state_is_final` (job_status, ignored) | A late post tried to rewrite a finished job. | Informational. |
| `queue_poisoned` / `queue_inconsistent` | The app's internal queue lock/state broke — should never happen. | Restart the app and export diagnostics. |
| `environment_report_too_large` | A page sent an oversized environment probe; rejected. | Informational (abuse guard). |

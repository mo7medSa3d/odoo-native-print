# Research: IPP (RFC 8010 encoding, RFC 8011 model)

Date accessed: 2026-11-03. Sources: PWG "How to Use IPP" guide
(istopwg.github.io), RFC 8010/8011 attribute and operation registries,
OpenPrinting/CUPS issue tracker, production IPP codec tests found via
live search.

## Wire format (RFC 8010)
- Message: version (1.1 = 0x0101), operation-id/status (2 bytes),
  request-id (4 bytes), then attribute groups, then 0x03
  (end-of-attributes), then document bytes for Print-Job.
- Group delimiters: operation 0x01, job 0x02, printer 0x04, unsupported 0x05.
- Value tags used here: integer 0x21, boolean 0x22, enum 0x23,
  textWithoutLanguage 0x41, nameWithoutLanguage 0x42, keyword 0x44,
  uri 0x45, charset 0x47, naturalLanguage 0x48, mimeMediaType 0x49.
- Each attribute: tag, name-length, name, value-length, value. A zero
  name-length means "another value for the previous attribute" (1setOf).
- First two operation attributes must be attributes-charset and
  attributes-natural-language, in that order.
- Operations: Print-Job 0x0002, Get-Job-Attributes 0x0009,
  Get-Printer-Attributes 0x000B, Cancel-Job 0x0008.
- Status: successful-ok 0x0000. An IPP error is carried in the IPP status
  code over HTTP 200, not in the HTTP status.
- printer-state enum: 3 idle, 4 processing, 5 stopped.
  job-state enum: 3 pending, 4 pending-held, 5 processing, 6 stopped,
  7 canceled, 8 aborted, 9 completed.
- Transport: HTTP POST, Content-Type: application/ipp, to the printer URI
  (commonly http://host:631/ipp/print).

## Pitfalls verified from the field
- document-format is a single-valued operation attribute: sending it
  twice in a Print-Job makes strict printers reject the job (CUPS issue
  #1457). Our encoder emits it exactly once, in the job group only when
  set there — we put it in the operation group once, nowhere else.
- Many printers do NOT accept application/pdf. The client must read
  document-format-supported via Get-Printer-Attributes first and pick:
  PDF if listed, else the best raster the printer accepts (PWG Raster,
  URF, JPEG). Our print-ipp crate implements the query and the choice
  function; raster conversion itself is the render pipeline's job.

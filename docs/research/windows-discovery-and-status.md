# Research: Windows printer discovery and status sensors

Date accessed: 2026-11-03 (second research pass, per user request).
Sources: Microsoft Win32 printing documentation as mirrored in the Win32
API reference (PRINTER_INFO_2 structure, winspool.h), EnumPrinters usage
in production code samples found via live search.

## Discovery
- EnumPrintersW with PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS at
  level 2 returns every local, per-user connected, and shared printer with
  name, share name, port, driver, attributes, status, and queued job count
  in one call. This is the discovery API; GetPrinterW on an open handle
  returns the same level-2 structure for one printer (the "sensor read").
- PRINTER_INFO_2 field order (verified): pServerName, pPrinterName,
  pShareName, pPortName, pDriverName, pComment, pLocation, pDevMode,
  pSepFile, pPrintProcessor, pDatatype, pParameters, pSecurityDescriptor,
  Attributes, Priority, DefaultPriority, StartTime, UntilTime, Status,
  cJobs, AveragePPM.
- is_default comes from Attributes & PRINTER_ATTRIBUTE_DEFAULT (0x0004);
  a printer Windows considers unusable offline also carries status flags.

## Status flags (PRINTER_STATUS_*, verified values)
PAUSED 0x1, ERROR 0x2, PENDING_DELETION 0x4, PAPER_JAM 0x8,
PAPER_OUT 0x10, MANUAL_FEED 0x20, PAPER_PROBLEM 0x40, OFFLINE 0x80,
IO_ACTIVE 0x100, BUSY 0x200, PRINTING 0x400, OUTPUT_BIN_FULL 0x800,
NOT_AVAILABLE 0x1000, WAITING 0x2000, PROCESSING 0x4000,
INITIALIZING 0x8000, WARMING_UP 0x10000, TONER_LOW 0x20000,
NO_TONER 0x40000, PAGE_PUNT 0x80000, USER_INTERVENTION 0x100000,
OUT_OF_MEMORY 0x200000, DOOR_OPEN 0x400000, SERVER_UNKNOWN 0x800000,
POWER_SAVE 0x1000000.
Our decode (print-core::printer) prioritises the conditions a user must
act on: door open, paper jam, paper out, no toner, offline, paused,
error; then activity states (printing/processing/busy); toner low is a
warning flag layered on the primary state.

## Change notification
FindFirstPrinterChangeNotification / FindNextPrinterChangeNotification
(PRINTER_CHANGE_ADD_PRINTER / DELETE / SET_PRINTER) is the spooler
notification mechanism for re-enumeration; polling GetPrinter at a low
frequency is the fallback the worker uses for status sensors. Both are
recorded here; the polling path ships first (simpler, sufficient for
job routing decisions), notifications are the optimisation.

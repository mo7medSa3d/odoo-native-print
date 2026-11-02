//! print-ipp: a dependency-free IPP client (RFC 8010 encoding,
//! RFC 8011 model) for driverless network printing.
//!
//! Used for: Windows fallback for network printers without a usable
//! driver, and the Android phase's silent printing path. The workflow
//! is always: Get-Printer-Attributes first (never assume a printer
//! accepts PDF), choose a format the printer actually lists, then
//! Print-Job with that single document-format value.

pub mod client;
pub mod proto;

pub use client::{IppClient, IppError, JobInfo, PrinterCaps};
pub use proto::{parse_message, Attr, IppMessage, Value};

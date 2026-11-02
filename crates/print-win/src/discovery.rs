//! Windows printer discovery and status sensors (cfg(windows) FFI in
//! win.rs reads the structures; the layout and conversion logic here is
//! platform-independent and unit-tested everywhere).
//!
//! PRINTER_INFO_2 (winspool.h, verified 2026-11-03 — see
//! docs/research/windows-discovery-and-status.md). On 64-bit Windows the
//! struct is 13 pointers followed by 8 DWORDs; Status sits at byte
//! offset 124 and cJobs at 128. We read through a repr(C) mirror so the
//! compiler, not a hand-computed offset, does the layout on Windows.

#[cfg(test)]
use print_core::printer::PrinterKind;
use print_core::printer::{decode_printer_status, PrinterInfo};

/// A raw level-2 record as read from Windows, before conversion.
#[derive(Debug, Clone, PartialEq)]
pub struct RawPrinterInfo2 {
    pub server_name: String,
    pub printer_name: String,
    pub share_name: String,
    pub port_name: String,
    pub driver_name: String,
    pub attributes: u32,
    pub status: u32,
    pub job_count: u32,
}

pub const PRINTER_ATTRIBUTE_DEFAULT: u32 = 0x0000_0004;
pub const PRINTER_ATTRIBUTE_SHARED: u32 = 0x0000_0008;
pub const PRINTER_ATTRIBUTE_WORK_OFFLINE: u32 = 0x0000_0400;

pub fn convert(raw: &RawPrinterInfo2) -> PrinterInfo {
    let mut status = decode_printer_status(raw.status);
    if raw.attributes & PRINTER_ATTRIBUTE_WORK_OFFLINE != 0
        && status == print_core::PrinterStatus::Ready
    {
        status = print_core::PrinterStatus::Offline;
    }
    PrinterInfo {
        kind: PrinterInfo::classify(&raw.printer_name, &raw.port_name),
        name: raw.printer_name.clone(),
        driver: raw.driver_name.clone(),
        port: raw.port_name.clone(),
        status,
        is_default: raw.attributes & PRINTER_ATTRIBUTE_DEFAULT != 0,
    }
}

impl From<&RawPrinterInfo2> for PrinterInfo {
    fn from(raw: &RawPrinterInfo2) -> Self {
        convert(raw)
    }
}

/// Is this a printer a user would expect to see (filters out the
/// pseudo-printers some drivers register with no port)?
pub fn is_usable(raw: &RawPrinterInfo2) -> bool {
    !raw.printer_name.is_empty() && !raw.port_name.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use print_core::PrinterStatus;

    fn raw(status: u32, attrs: u32) -> RawPrinterInfo2 {
        RawPrinterInfo2 {
            server_name: String::new(),
            printer_name: "HP LaserJet Pro".into(),
            share_name: String::new(),
            port_name: "USB001".into(),
            driver_name: "HP Universal".into(),
            attributes: attrs,
            status,
            job_count: 0,
        }
    }
    #[test]
    fn convert_maps_status_and_default() {
        let info = convert(&raw(0x10, PRINTER_ATTRIBUTE_DEFAULT));
        assert_eq!(info.status, PrinterStatus::PaperOut);
        assert!(info.is_default);
        assert_eq!(info.kind, PrinterKind::Local);
    }
    #[test]
    fn work_offline_attribute_forces_offline() {
        let info = convert(&raw(0, PRINTER_ATTRIBUTE_WORK_OFFLINE));
        assert_eq!(info.status, PrinterStatus::Offline);
    }
    #[test]
    fn usable_filter() {
        assert!(is_usable(&raw(0, 0)));
        let mut r = raw(0, 0);
        r.port_name = String::new();
        assert!(!is_usable(&r));
    }
}

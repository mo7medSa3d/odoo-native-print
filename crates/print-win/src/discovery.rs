//! Windows printer discovery and status sensors (cfg(windows) FFI in
//! win.rs reads the structures; the layout and conversion logic here is
//! platform-independent and unit-tested everywhere).
//!
//! PRINTER_INFO_2 (winspool.h, verified 2026-11-03 — see
//! docs/research/windows-discovery-and-status.md). On 64-bit Windows the
//! struct is 13 pointers followed by 8 DWORDs; Status sits at byte
//! offset 124 and cJobs at 128. We read through a repr(C) mirror so the
//! compiler, not a hand-computed offset, does the layout on Windows.

use print_core::paper::Form;
use print_core::printer::{decode_printer_status, PrinterInfo, PrinterKind};
use print_core::PageSize;

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
    // A UNC queue (\\\\server\\printer) is a printer shared by another
    // workstation/server, not a device attached to this workstation.
    // Keep directly installed network printers (TCP/IP, WSD, IPP ports):
    // their local queue name is ordinary text and their port is network-backed.
    !raw.printer_name.is_empty()
        && !raw.port_name.is_empty()
        && PrinterInfo::classify(&raw.printer_name, &raw.port_name) != PrinterKind::Shared
}

/// Convert a driver-reported paper name and size (mm) into a `Form`,
/// detecting continuous thermal rolls by name or standard roll geometry.
pub fn classify_form(raw_name: &str, width_mm: f64, height_mm: f64) -> Form {
    let trimmed = raw_name.trim();
    let name = if trimmed.is_empty() {
        format!("{width_mm:.0}x{height_mm:.0}mm")
    } else {
        trimmed.to_string()
    };
    let lower = name.to_ascii_lowercase();
    let is_roll_width = (width_mm - 80.0).abs() <= 4.0
        || (width_mm - 72.0).abs() <= 4.0
        || (width_mm - 58.0).abs() <= 4.0;
    let is_roll = lower.contains("roll")
        || lower.contains("receipt")
        || lower.contains("continuous")
        || (is_roll_width && height_mm >= 200.0 && (height_mm - 297.0).abs() > 1.5);
    if is_roll {
        let canonical_w = if (width_mm - 72.0).abs() <= 4.0 {
            80.0
        } else {
            width_mm
        };
        Form {
            name,
            size: PageSize::new(canonical_w, height_mm),
            roll: true,
            paper_id: None,
        }
    } else {
        Form::sheet(&name, PageSize::new(width_mm, height_mm))
    }
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
    #[test]
    fn excludes_remote_unc_shared_queues_but_keeps_local_network_queues() {
        let mut remote = raw(0, 0);
        remote.printer_name = r"\\print-server\receipt".into();
        remote.port_name = "Ne03:".into();
        assert!(!is_usable(&remote));

        let mut network = raw(0, 0);
        network.printer_name = "Warehouse Receipt Printer".into();
        network.port_name = "IP_192.168.10.25".into();
        assert!(is_usable(&network));
    }

    #[test]
    fn classify_sheet_and_roll_forms() {
        let a4 = classify_form("A4", 210.0, 297.0);
        assert!(!a4.roll);
        assert_eq!(a4.name, "A4");

        let roll = classify_form("80 x 3276 mm Roll", 79.5, 3276.0);
        assert!(roll.roll);
        assert!((roll.size.width_mm - 79.5).abs() < 0.1);

        let fallback = classify_form("", 100.0, 150.0);
        assert_eq!(fallback.name, "100x150mm");
        assert!(!fallback.roll);
    }
}

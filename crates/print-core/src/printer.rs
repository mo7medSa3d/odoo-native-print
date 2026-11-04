use crate::paper::Form;
use serde::{Deserialize, Serialize};

/// Printer status decoded from Win32 PRINTER_STATUS_* flags
/// (winspool.h). Values are the documented Microsoft constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrinterStatus {
    Ready,
    Offline,
    PaperOut,
    PaperJam,
    DoorOpen,
    NoToner,
    Paused,
    Printing,
    Error,
    UserIntervention,
    Unknown,
}

/// Toner low (0x20000) is a warning, not a state: a printer can be
/// Ready with low toner. Exposed separately so UIs can badge it.
pub fn has_toner_low(flags: u32) -> bool {
    flags & 0x0002_0000 != 0
}

/// True when a human must act before this printer can print.
pub fn needs_intervention(s: PrinterStatus) -> bool {
    matches!(
        s,
        PrinterStatus::Offline
            | PrinterStatus::PaperOut
            | PrinterStatus::PaperJam
            | PrinterStatus::DoorOpen
            | PrinterStatus::NoToner
            | PrinterStatus::Paused
            | PrinterStatus::Error
            | PrinterStatus::UserIntervention
    )
}

pub fn decode_printer_status(flags: u32) -> PrinterStatus {
    const PAUSED: u32 = 0x0000_0001;
    const ERROR: u32 = 0x0000_0002;
    const PAPER_JAM: u32 = 0x0000_0008;
    const PAPER_OUT: u32 = 0x0000_0010;
    const OFFLINE: u32 = 0x0000_0080;
    const PRINTING: u32 = 0x0000_0400;
    const PROCESSING: u32 = 0x0000_4000;
    const NO_TONER: u32 = 0x0004_0000;
    const DOOR_OPEN: u32 = 0x0040_0000;
    const PAPER_PROBLEM: u32 = 0x0000_0040;
    const BUSY: u32 = 0x0000_0200;
    const MANUAL_FEED: u32 = 0x0000_0020;
    const NOT_AVAILABLE: u32 = 0x0000_1000;
    const OUTPUT_BIN_FULL: u32 = 0x0000_0800;
    const USER_INTERVENTION: u32 = 0x0010_0000;
    // Priority: what the user must fix first, then activity states.
    if flags & DOOR_OPEN != 0 {
        PrinterStatus::DoorOpen
    } else if flags & PAPER_JAM != 0 {
        PrinterStatus::PaperJam
    } else if flags & PAPER_OUT != 0 {
        PrinterStatus::PaperOut
    } else if flags & NO_TONER != 0 {
        PrinterStatus::NoToner
    } else if flags & OFFLINE != 0 || flags & NOT_AVAILABLE != 0 {
        PrinterStatus::Offline
    } else if flags & USER_INTERVENTION != 0
        || flags & MANUAL_FEED != 0
        || flags & OUTPUT_BIN_FULL != 0
    {
        // A human must act (or the device needs attention) before
        // anything can print — never let these fall through to Unknown,
        // which every consumer treats as printable.
        PrinterStatus::UserIntervention
    } else if flags & PAPER_PROBLEM != 0 {
        PrinterStatus::Error
    } else if flags & PAUSED != 0 {
        PrinterStatus::Paused
    } else if flags & ERROR != 0 {
        PrinterStatus::Error
    } else if flags & (PRINTING | PROCESSING | BUSY) != 0 {
        PrinterStatus::Printing
    } else if flags == 0 {
        PrinterStatus::Ready
    } else {
        PrinterStatus::Unknown
    }
}

fn is_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.chars().all(|c| c.is_ascii_digit())
                && p.parse::<u16>().map(|v| v <= 255).unwrap_or(false)
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrinterKind {
    Local,
    Shared,  // UNC \\server\printer
    Network, // IPP / TCP / WSD port
    Virtual, // PDF / XPS writers
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterCapabilities {
    pub forms: Vec<Form>,
    pub resolutions_dpi: Vec<u32>,
    pub duplex: bool,
    pub color: bool,
    pub bins: Vec<String>,
    pub max_copies: u16,
    pub custom_size_supported: bool,
    pub min_custom_mm: Option<(f64, f64)>,
    pub max_custom_mm: Option<(f64, f64)>,
}

impl PrinterCapabilities {
    /// Can this printer physically take a custom size of `size`?
    pub fn custom_size_ok(&self, size: crate::PageSize) -> bool {
        if !self.custom_size_supported {
            return false;
        }
        let fits_orientation = |width: f64, height: f64| {
            if let Some((min_w, min_h)) = self.min_custom_mm {
                if width < min_w || height < min_h {
                    return false;
                }
            }
            if let Some((max_w, max_h)) = self.max_custom_mm {
                if width > max_w || height > max_h {
                    return false;
                }
            }
            true
        };
        fits_orientation(size.width_mm, size.height_mm)
            || fits_orientation(size.height_mm, size.width_mm)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterInfo {
    pub name: String,
    pub driver: String,
    pub port: String,
    pub kind: PrinterKind,
    pub status: PrinterStatus,
    pub is_default: bool,
}

impl PrinterInfo {
    pub fn classify(name: &str, port: &str) -> PrinterKind {
        let p = port.to_ascii_uppercase();
        let n = name.to_ascii_lowercase();
        if n.contains("print to pdf") || n.contains("xps") || p.starts_with("PORTPROMPT") {
            PrinterKind::Virtual
        } else if name.starts_with("\\\\") {
            PrinterKind::Shared
        } else if p.starts_with("USB")
            || p.starts_with("COM")
            || p.starts_with("LPT")
            || p.starts_with("FILE:")
        {
            // Physical local ports and print-to-file: the ':' in
            // "COM1:" / "FILE:" must not read as a network endpoint.
            PrinterKind::Local
        } else if p.contains("IP_")
            || p.contains("IPP")
            || p.contains("WSD")
            || p.contains(':')
            || is_ipv4(&p)
        {
            PrinterKind::Network
        } else {
            PrinterKind::Local
        }
    }
}

/// True when a job must NOT be delivered to this printer class: raw
/// printer languages (ESC/POS, ZPL, TSPL, EPL — whether the payload
/// itself is raw or a PDF/image would be converted to raw) can only
/// go to a physical printer. A virtual PDF/XPS writer would spool the
/// raw bytes into a document file, producing a corrupt, unopenable
/// "PDF" instead of a page (field report, 2026-11-04).
pub fn raw_delivery_blocked(payload_kind: &str, mode: &str, printer: PrinterKind) -> bool {
    if printer != PrinterKind::Virtual {
        return false;
    }
    let raw = |v: &str| matches!(v, "escpos" | "zpl" | "tspl" | "epl");
    raw(payload_kind) || raw(mode)
}

/// Whether a job should be re-routed to GDI printing instead of
/// being rejected by `raw_delivery_blocked`. An IMAGE addressed to
/// a virtual printer in a raw-language mode (a POS receipt whose
/// target resolved to a PDF writer, say) still has a faithful
/// rendering: the same bitmap, printed through GDI at the paper
/// size, like any office image. Failing the job instead made the
/// POS fall back to the browser print dialog in the field
/// (2026-11-04). Raw payloads — actual printer-language bytes —
/// have no GDI rendering and stay hard-blocked.
pub fn image_mode_downgrade(
    payload_kind: &str,
    mode: &str,
    printer: PrinterKind,
) -> Option<&'static str> {
    if printer != PrinterKind::Virtual {
        return None;
    }
    if payload_kind != "image-png" {
        return None;
    }
    match mode {
        "escpos" | "zpl" | "tspl" | "epl" => Some("gdi"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intervention_flags_decode() {
        // Audit D-2: flags that used to fall through to Unknown
        // (treated as printable by every gate).
        assert_eq!(
            decode_printer_status(0x0010_0000),
            PrinterStatus::UserIntervention
        );
        assert!(needs_intervention(PrinterStatus::UserIntervention));
        assert_eq!(
            decode_printer_status(0x0000_0020),
            PrinterStatus::UserIntervention
        ); // manual feed
        assert_eq!(
            decode_printer_status(0x0000_0800),
            PrinterStatus::UserIntervention
        ); // output bin full
        assert_eq!(decode_printer_status(0x0000_0040), PrinterStatus::Error); // paper problem
        assert_eq!(decode_printer_status(0x0000_1000), PrinterStatus::Offline); // not available
        assert_eq!(decode_printer_status(0x0000_0200), PrinterStatus::Printing);
        // busy
    }

    #[test]
    fn status_priority() {
        assert_eq!(decode_printer_status(0), PrinterStatus::Ready);
        assert_eq!(decode_printer_status(0x80), PrinterStatus::Offline);
        assert_eq!(decode_printer_status(0x10), PrinterStatus::PaperOut);
        // Door open wins over paper out when both bits are set.
        assert_eq!(
            decode_printer_status(0x0040_0000 | 0x10),
            PrinterStatus::DoorOpen
        );
        assert_eq!(decode_printer_status(0x8), PrinterStatus::PaperJam);
        assert_eq!(decode_printer_status(0x0004_0000), PrinterStatus::NoToner);
        assert_eq!(decode_printer_status(0x400), PrinterStatus::Printing);
        assert!(has_toner_low(0x0002_0000));
        assert!(!has_toner_low(0));
        assert!(needs_intervention(PrinterStatus::PaperJam));
        assert!(!needs_intervention(PrinterStatus::Printing));
    }
    #[test]
    fn classify_local_ports() {
        // Audit L11: ':' in a local port name is not a network endpoint.
        assert_eq!(
            PrinterInfo::classify("HP LaserJet", "COM1:"),
            PrinterKind::Local
        );
        assert_eq!(
            PrinterInfo::classify("HP LaserJet", "LPT1:"),
            PrinterKind::Local
        );
        assert_eq!(
            PrinterInfo::classify("HP LaserJet", "USB001"),
            PrinterKind::Local
        );
        assert_eq!(
            PrinterInfo::classify("HP LaserJet", "FILE:"),
            PrinterKind::Local
        );
    }

    #[test]
    fn classify_kinds() {
        assert_eq!(
            PrinterInfo::classify("Microsoft Print to PDF", "PORTPROMPT:"),
            PrinterKind::Virtual
        );
        assert_eq!(
            PrinterInfo::classify("\\\\SERVER\\HP", "TS001"),
            PrinterKind::Shared
        );
        assert_eq!(
            PrinterInfo::classify("HP", "192.168.1.50"),
            PrinterKind::Network
        );
        assert_eq!(PrinterInfo::classify("HP", "USB001"), PrinterKind::Local);
    }
    #[test]
    fn custom_bounds() {
        let caps = PrinterCapabilities {
            forms: vec![],
            resolutions_dpi: vec![300],
            duplex: false,
            color: false,
            bins: vec![],
            max_copies: 1,
            custom_size_supported: true,
            min_custom_mm: Some((20.0, 20.0)),
            max_custom_mm: Some((210.0, 297.0)),
        };
        assert!(caps.custom_size_ok(crate::PageSize::label_100x150()));
        assert!(!caps.custom_size_ok(crate::PageSize::new(300.0, 300.0)));
        assert!(!caps.custom_size_ok(crate::PageSize::new(10.0, 100.0)));
    }

    #[test]
    fn image_mode_downgrade_only_for_images_to_virtual_printers() {
        use super::image_mode_downgrade;
        // A receipt image in a raw mode to a virtual writer re-routes
        // to GDI instead of failing.
        assert_eq!(
            image_mode_downgrade("image-png", "escpos", PrinterKind::Virtual),
            Some("gdi")
        );
        assert_eq!(
            image_mode_downgrade("image-png", "zpl", PrinterKind::Virtual),
            Some("gdi")
        );
        // Already-GDI images need nothing.
        assert_eq!(
            image_mode_downgrade("image-png", "gdi", PrinterKind::Virtual),
            None
        );
        // Documents and raw payloads are NOT downgraded: reports keep
        // their loud block, and printer-language bytes have no GDI
        // rendering at all.
        assert_eq!(
            image_mode_downgrade("pdf", "escpos", PrinterKind::Virtual),
            None
        );
        assert_eq!(
            image_mode_downgrade("zpl", "zpl", PrinterKind::Virtual),
            None
        );
        // Physical printers are never re-routed.
        assert_eq!(
            image_mode_downgrade("image-png", "escpos", PrinterKind::Local),
            None
        );
        assert_eq!(
            image_mode_downgrade("image-png", "escpos", PrinterKind::Network),
            None
        );
    }

    #[test]
    fn raw_delivery_blocked_only_for_virtual_printers() {
        use super::raw_delivery_blocked;
        // Raw payloads and raw conversions are blocked on virtual writers.
        assert!(raw_delivery_blocked(
            "escpos",
            "escpos",
            PrinterKind::Virtual
        ));
        assert!(raw_delivery_blocked("zpl", "zpl", PrinterKind::Virtual));
        assert!(raw_delivery_blocked("pdf", "escpos", PrinterKind::Virtual));
        assert!(raw_delivery_blocked(
            "image-png",
            "tspl",
            PrinterKind::Virtual
        ));
        // GDI documents to a virtual writer are the legitimate preview path.
        assert!(!raw_delivery_blocked("pdf", "gdi", PrinterKind::Virtual));
        // Physical printers take everything.
        assert!(!raw_delivery_blocked(
            "escpos",
            "escpos",
            PrinterKind::Local
        ));
        assert!(!raw_delivery_blocked("zpl", "zpl", PrinterKind::Network));
        assert!(!raw_delivery_blocked("pdf", "escpos", PrinterKind::Shared));
    }
}

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
    /// Detected print protocol and device class from the driver
    /// identity (`detect_protocol`): "gdi"/"escpos"/"zpl"/"tspl"/
    /// "epl" and "office"/"thermal"/"label"/"virtual". Additive
    /// bridge fields so every consumer (Control Center cards, test
    /// lab, Odoo pages) offers only what the printer actually
    /// speaks (owner, 2026-11-04).
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub printer_class: String,
}

/// Conservative protocol inference from the installed driver
/// identity — the Rust twin of the addon JS `detectPrinterType`
/// (native_print_client.js); the two tables must stay in step.
/// Short protocol tokens match on word boundaries only: a bare
/// substring search classifies any name containing "epl"
/// ("Repl…") as a raw label printer.
pub fn detect_protocol(
    name: &str,
    driver: &str,
    kind: PrinterKind,
) -> (&'static str, &'static str) {
    if kind == PrinterKind::Virtual {
        return ("gdi", "virtual");
    }
    let signature = format!("{} {}", name, driver).to_lowercase();
    let has_word = |tok: &str| -> bool {
        let bytes = signature.as_bytes();
        let t = tok.as_bytes();
        if t.is_empty() || bytes.len() < t.len() {
            return false;
        }
        (0..=bytes.len() - t.len()).any(|i| {
            bytes[i..i + t.len()] == *t
                && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
                && (i + t.len() == bytes.len() || !bytes[i + t.len()].is_ascii_alphanumeric())
        })
    };
    if signature.contains("zdesigner") || signature.contains("zebra") || has_word("zpl") {
        return ("zpl", "label");
    }
    if has_word("tsc") || has_word("tspl") {
        return ("tspl", "label");
    }
    if has_word("epl") {
        return ("epl", "label");
    }
    const THERMAL_TOKENS: [&str; 12] = [
        "epson tm-",
        "epson tm ",
        "xprinter",
        "rongta",
        "bixolon",
        "star tsp",
        "citizen ct-",
        "sewoo",
        "esc/pos",
        "escpos",
        "thermal receipt",
        "receipt printer",
    ];
    if THERMAL_TOKENS.iter().any(|t| signature.contains(t)) {
        return ("escpos", "thermal");
    }
    // "pos-58"/"pos-80" from the JS table are substring tokens for
    // generic receipt printers; they stay substring matches there,
    // so they stay substring matches here.
    if signature.contains("pos-58") || signature.contains("pos-80") {
        return ("escpos", "thermal");
    }
    if kind == PrinterKind::Network && (signature.contains("ipp") || signature.contains("airprint"))
    {
        return ("gdi", "office");
    }
    ("gdi", "office")
}

impl PrinterInfo {
    pub fn classify(name: &str, port: &str) -> PrinterKind {
        let p = port.to_ascii_uppercase();
        let n = name.to_ascii_lowercase();
        // Virtual writers render into a file or another app instead
        // of paper: PDF/XPS writers, the save-prompt port, OneNote
        // (its port is literally "nul:" — the 2026-11-04 field
        // screenshot showed OneNote classified as NETWORK, which
        // offered it raw test prints it cannot accept), and any
        // other printer parked on the NUL discard port.
        if n.contains("print to pdf")
            || n.contains("xps")
            || n.contains("onenote")
            || p.starts_with("PORTPROMPT")
            || p.starts_with("NUL:")
        {
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
/// being rejected by `raw_delivery_blocked`. A DOCUMENT (PDF or
/// image) addressed to a virtual printer in a raw-language mode
/// still has a faithful rendering: the same pages, printed through
/// GDI at the printer's paper size — which is how virtual printers
/// always worked in this product (OneNote opens with the printed
/// page, a PDF writer produces the document). The owner ruled
/// virtual printers first-class silent targets (2026-11-04): a
/// document job must never die just because its target is virtual.
/// Raw payloads — actual printer-language bytes — have no GDI
/// rendering and stay hard-blocked by `raw_delivery_blocked`.
pub fn document_mode_downgrade(
    payload_kind: &str,
    mode: &str,
    printer: PrinterKind,
) -> Option<&'static str> {
    if printer != PrinterKind::Virtual {
        return None;
    }
    if !matches!(payload_kind, "pdf" | "image-png") {
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
    fn classify_treats_onenote_and_nul_port_as_virtual() {
        // Field screenshot, 2026-11-04: OneNote (Desktop) on port
        // "nul:" was classified Network, so its Control Center card
        // offered raw ESC/POS and ZPL test prints it cannot accept.
        assert_eq!(
            PrinterInfo::classify("OneNote (Desktop)", "nul:"),
            PrinterKind::Virtual
        );
        assert_eq!(
            PrinterInfo::classify("OneNote for Windows 10", "SHARING:"),
            PrinterKind::Virtual
        );
        assert_eq!(
            PrinterInfo::classify("Discard Queue", "NUL:"),
            PrinterKind::Virtual
        );
        // The established cases keep their kinds.
        assert_eq!(
            PrinterInfo::classify("Microsoft Print to PDF", "PORTPROMPT:"),
            PrinterKind::Virtual
        );
        assert_eq!(
            PrinterInfo::classify("HP LaserJet", "USB001"),
            PrinterKind::Local
        );
        assert_eq!(
            PrinterInfo::classify("EPSON TM-T88VII", "192.168.1.50"),
            PrinterKind::Network
        );
    }

    #[test]
    fn detect_protocol_mirrors_the_driver_identity_table() {
        use super::detect_protocol;
        // Virtual writers are GDI targets whatever their names say.
        assert_eq!(
            detect_protocol(
                "OneNote (Desktop)",
                "Send to Microsoft OneNote 16 Driver",
                PrinterKind::Virtual
            ),
            ("gdi", "virtual")
        );
        assert_eq!(
            detect_protocol(
                "Microsoft Print to PDF",
                "Microsoft Print To PDF",
                PrinterKind::Virtual
            ),
            ("gdi", "virtual")
        );
        // Raw languages by driver identity (word boundaries: the
        // "Replit" office printer must not read as EPL).
        assert_eq!(
            detect_protocol("EPSON TM-T88VII", "EPSON", PrinterKind::Network),
            ("escpos", "thermal")
        );
        assert_eq!(
            detect_protocol("Zebra ZPL Label", "Zebra", PrinterKind::Local),
            ("zpl", "label")
        );
        assert_eq!(
            detect_protocol("TSC TTP-244", "TSC", PrinterKind::Local),
            ("tspl", "label")
        );
        assert_eq!(
            detect_protocol("Replit OfficeJet", "HP", PrinterKind::Local),
            ("gdi", "office")
        );
        assert_eq!(
            detect_protocol("HP LaserJet", "HP PCL", PrinterKind::Local),
            ("gdi", "office")
        );
        assert_eq!(
            detect_protocol("POS-80 Receipt", "Generic", PrinterKind::Local),
            ("escpos", "thermal")
        );
    }

    #[test]
    fn document_mode_downgrade_only_for_documents_to_virtual_printers() {
        use super::document_mode_downgrade;
        // A receipt image or a report PDF in a raw mode to a virtual
        // writer re-routes to GDI instead of failing: the owner
        // ruled virtual printers first-class silent targets.
        assert_eq!(
            document_mode_downgrade("image-png", "escpos", PrinterKind::Virtual),
            Some("gdi")
        );
        assert_eq!(
            document_mode_downgrade("pdf", "escpos", PrinterKind::Virtual),
            Some("gdi")
        );
        assert_eq!(
            document_mode_downgrade("pdf", "zpl", PrinterKind::Virtual),
            Some("gdi")
        );
        // Already-GDI documents need nothing.
        assert_eq!(
            document_mode_downgrade("image-png", "gdi", PrinterKind::Virtual),
            None
        );
        assert_eq!(
            document_mode_downgrade("pdf", "gdi", PrinterKind::Virtual),
            None
        );
        // Raw payloads are NOT downgraded: printer-language bytes
        // have no GDI rendering at all and stay hard-blocked.
        assert_eq!(
            document_mode_downgrade("zpl", "zpl", PrinterKind::Virtual),
            None
        );
        assert_eq!(
            document_mode_downgrade("escpos", "escpos", PrinterKind::Virtual),
            None
        );
        // Physical printers are never re-routed.
        assert_eq!(
            document_mode_downgrade("image-png", "escpos", PrinterKind::Local),
            None
        );
        assert_eq!(
            document_mode_downgrade("pdf", "escpos", PrinterKind::Network),
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

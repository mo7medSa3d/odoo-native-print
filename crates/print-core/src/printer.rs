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
    /// How each capability value was obtained. A value reported by
    /// the driver is evidence; a value defaulted because the driver
    /// call failed (`DeviceCapabilitiesW` returning -1, which the
    /// API does not distinguish from "unsupported") is `Unknown`
    /// and must not be presented as a confirmed device fact.
    /// Absent in older serialized payloads: defaults to all-unknown.
    #[serde(default)]
    pub provenance: CapabilityReport,
}

/// Per-field provenance for [`PrinterCapabilities`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityProvenance {
    /// Reported by the driver / device protocol.
    DriverReported,
    /// Derived by the application from other evidence.
    Inferred,
    /// Set by the operator.
    Configured,
    /// Not reported, or the capability query itself failed: the
    /// value beside it (if any) is a fallback, not a device fact.
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CapabilityReport {
    #[serde(default)]
    pub duplex: CapabilityProvenance,
    #[serde(default)]
    pub color: CapabilityProvenance,
    #[serde(default)]
    pub max_copies: CapabilityProvenance,
    #[serde(default)]
    pub resolutions: CapabilityProvenance,
    #[serde(default)]
    pub bins: CapabilityProvenance,
    #[serde(default)]
    pub custom_size: CapabilityProvenance,
}

impl Default for PrinterCapabilities {
    fn default() -> Self {
        Self {
            forms: Vec::new(),
            resolutions_dpi: Vec::new(),
            duplex: false,
            color: false,
            bins: Vec::new(),
            max_copies: 1,
            custom_size_supported: false,
            min_custom_mm: None,
            max_custom_mm: None,
            provenance: CapabilityReport::default(),
        }
    }
}

fn provenance_of(reported: bool) -> CapabilityProvenance {
    if reported {
        CapabilityProvenance::DriverReported
    } else {
        CapabilityProvenance::Unknown
    }
}

/// Render DPI to use for raw (raster) output given the resolutions
/// the driver reported. 203 dpi is the thermal/label raster standard
/// and is preferred when reported; otherwise the smallest reported
/// resolution in the 200-300 dpi band (rendering a receipt at 600 dpi
/// wastes memory and buffer for no physical gain); otherwise the
/// smallest reported resolution; with no report at all, 203 dpi is
/// the documented fallback, not a claimed device fact.
pub fn preferred_raw_render_dpi(resolutions_dpi: &[u32]) -> u32 {
    if resolutions_dpi.contains(&203) {
        return 203;
    }
    if let Some(dpi) = resolutions_dpi
        .iter()
        .copied()
        .filter(|d| (200..=300).contains(d))
        .min()
    {
        return dpi;
    }
    resolutions_dpi.iter().copied().min().unwrap_or(203)
}

/// Stable identity for a spooler queue backed by a physical
/// endpoint, or `None` when the port cannot identify a device
/// (virtual writers, print-to-file, generic local ports).
///
/// The fingerprint hashes the normalized driver name and port: a
/// queue rename does not change it, and two queues on the same
/// physical port (one device, two queues) share it. It does NOT
/// survive a driver reinstall (the driver name is part of the
/// hash) or a USB port renumbering, because the spooler exposes no
/// stronger stable identifier (no device instance ID) through the
/// level-2 record used here. Callers must treat two different
/// fingerprints as "possibly the same device, not proven
/// different" only via this function — never merge devices on a
/// name or model match alone.
pub fn stable_printer_id(driver: &str, port: &str) -> Option<String> {
    let p = port.trim().to_ascii_uppercase();
    let endpoint = p.starts_with("USB")
        || p.starts_with("COM")
        || p.starts_with("LPT")
        || p.contains("IP_")
        || p.contains("WSD")
        || p.contains("IPP")
        || is_ipv4(&p);
    if !endpoint {
        return None;
    }
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in format!("{}|{}", driver.trim().to_ascii_lowercase(), p).as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Some(format!("spooler-{hash:016x}"))
}

/// True when both identities name the same physical endpoint.
/// `None` identities never merge: unknown identity is not identity.
pub fn same_physical_endpoint(a: &str, b: &str) -> bool {
    !a.is_empty() && a == b
}

impl PrinterCapabilities {
    /// Build capabilities from driver-query results where each
    /// `Option` is `Some` when the driver actually answered and
    /// `None` when the query failed or was unsupported. `None`
    /// values fall back to safe defaults *and* are flagged
    /// `Unknown` in the provenance record, so no consumer can
    /// mistake a fallback for a reported capability.
    #[allow(clippy::too_many_arguments)]
    pub fn from_driver_report(
        forms: Vec<Form>,
        resolutions_dpi: Vec<u32>,
        duplex: Option<bool>,
        color: Option<bool>,
        max_copies: Option<u16>,
        bins: Vec<String>,
        bins_reported: bool,
        min_custom_mm: Option<(f64, f64)>,
        max_custom_mm: Option<(f64, f64)>,
    ) -> Self {
        let custom_size_supported = min_custom_mm.is_some() && max_custom_mm.is_some();
        let resolutions_reported = !resolutions_dpi.is_empty();
        Self {
            forms,
            resolutions_dpi,
            duplex: duplex.unwrap_or(false),
            color: color.unwrap_or(false),
            bins,
            max_copies: max_copies.unwrap_or(1),
            custom_size_supported,
            min_custom_mm,
            max_custom_mm,
            provenance: CapabilityReport {
                duplex: provenance_of(duplex.is_some()),
                color: provenance_of(color.is_some()),
                max_copies: provenance_of(max_copies.is_some()),
                resolutions: provenance_of(resolutions_reported),
                bins: provenance_of(bins_reported),
                custom_size: provenance_of(custom_size_supported),
            },
        }
    }
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
    /// Stable endpoint identity from driver + port (see
    /// [`stable_printer_id`]). Empty when the queue's port cannot
    /// identify a physical device. The display name is NOT an
    /// identity: renames must not split one device into two.
    #[serde(default)]
    pub stable_id: String,
    /// Full multi-condition health observation, when the discovery
    /// path produced one. `None` means no live observation exists —
    /// status alone must then be read as spooler-reported, possibly
    /// stale, never as a physical sensor read.
    #[serde(default)]
    pub health: Option<crate::health::PrinterHealth>,
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
    // Kept in sync with the fallback heuristic in the addon's
    // native_print_client.js (this Rust detection is the primary:
    // it ships on every PrinterInfo and the page prefers it). The
    // generic POS-printer names matter as much as the brands —
    // clone drivers very often identify only as "POS Printer" /
    // "POS80" / "Thermal Printer", and missing them routed real
    // POS printers to GDI (owner report, 2026-11-05).
    const THERMAL_TOKENS: [&str; 27] = [
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
        "thermal printer",
        "receipt printer",
        "pos printer",
        "pos-58",
        "pos-80",
        "pos58",
        "pos80",
        "tm-t",
        "tm-m",
        "sunmi",
        "munbyn",
        "hprt",
        "cashino",
        "snbc",
        "gprinter",
        "sprt",
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
            provenance: Default::default(),
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
        // Generic clone-driver identities (owner report,
        // 2026-11-05): a POS printer whose driver says nothing
        // but "POS Printer" / "Thermal Printer" / "POS80" is a
        // thermal printer, not an office printer.
        for (name, driver) in [
            ("POS Printer", "POS Printer Driver"),
            ("Thermal Printer", "Generic Thermal"),
            ("POS80", "POS Printer Driver"),
            ("Cashier", "SUNMI POS Driver"),
            ("XP-58IIH", "MUNBYN"),
        ] {
            assert_eq!(
                detect_protocol(name, driver, PrinterKind::Local),
                ("escpos", "thermal"),
                "{name} / {driver}"
            );
        }
        // Near-miss names stay office: "PostScript" contains "pos"
        // but is not a POS printer.
        assert_eq!(
            detect_protocol("PostScript Printer", "Adobe PDF", PrinterKind::Local),
            ("gdi", "office")
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
    fn stable_id_survives_rename_and_shared_port_but_not_virtual_ports() {
        // A queue rename keeps driver+port: same identity.
        let a = stable_printer_id("EPSON TM-T88VII", "USB001").unwrap();
        let renamed = stable_printer_id("EPSON TM-T88VII", "USB001").unwrap();
        assert_eq!(a, renamed);
        assert!(same_physical_endpoint(&a, &renamed));
        // A second queue on the same physical port is the same endpoint.
        assert!(same_physical_endpoint(
            &a,
            &stable_printer_id("EPSON TM-T88VII", "usb001").unwrap()
        ));
        // A different port is a different endpoint; virtual/file ports
        // have no endpoint identity at all and never merge.
        assert_ne!(a, stable_printer_id("EPSON TM-T88VII", "USB002").unwrap());
        assert_eq!(
            stable_printer_id("Microsoft Print To PDF", "PORTPROMPT:"),
            None
        );
        assert_eq!(stable_printer_id("OneNote", "NUL:"), None);
        assert_eq!(stable_printer_id("HP", "FILE:"), None);
        assert!(!same_physical_endpoint("", ""));
    }

    #[test]
    fn driver_report_provenance_flags_unknown_values() {
        let caps = PrinterCapabilities::from_driver_report(
            vec![],
            vec![203, 300],
            None,
            Some(true),
            None,
            vec![],
            false,
            None,
            None,
        );
        assert_eq!(
            caps.provenance.duplex,
            CapabilityProvenance::Unknown,
            "a failed duplex query must not read as reported"
        );
        assert_eq!(caps.provenance.color, CapabilityProvenance::DriverReported);
        assert_eq!(caps.provenance.max_copies, CapabilityProvenance::Unknown);
        assert_eq!(caps.max_copies, 1, "fallback value, flagged unknown");
        assert_eq!(
            caps.provenance.resolutions,
            CapabilityProvenance::DriverReported
        );
        assert!(!caps.custom_size_supported);
        // Old serialized payloads without the field deserialize
        // with all-unknown provenance.
        let legacy = r#"{"forms":[],"resolutions_dpi":[],"duplex":false,"color":false,"bins":[],"max_copies":1,"custom_size_supported":false,"min_custom_mm":null,"max_custom_mm":null}"#;
        let back: PrinterCapabilities = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            back.provenance,
            CapabilityReport::default(),
            "legacy payloads must not gain invented provenance"
        );
    }

    #[test]
    fn preferred_raw_render_dpi_prefers_203_then_band() {
        assert_eq!(preferred_raw_render_dpi(&[300, 203, 600]), 203);
        assert_eq!(preferred_raw_render_dpi(&[300, 600]), 300);
        assert_eq!(preferred_raw_render_dpi(&[600]), 600);
        assert_eq!(preferred_raw_render_dpi(&[]), 203);
        assert_eq!(preferred_raw_render_dpi(&[150, 300]), 300);
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

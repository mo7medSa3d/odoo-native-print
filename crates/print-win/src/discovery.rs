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
use print_core::printer::PrinterInfo;
#[cfg(test)]
use print_core::printer::PrinterKind;
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
    let kind = PrinterInfo::classify(&raw.printer_name, &raw.port_name);
    let (protocol, printer_class) =
        print_core::printer::detect_protocol(&raw.printer_name, &raw.driver_name, kind);
    let health = print_core::PrinterHealth::from_spooler_flags(
        raw.status,
        raw.attributes & PRINTER_ATTRIBUTE_WORK_OFFLINE != 0,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    );
    // The single status is derived from the full observation so the
    // two can never disagree (e.g. PAPER_PROBLEM reads as
    // UserIntervention in both, never Error in one and
    // UserIntervention in the other).
    let status = health.primary_status();
    PrinterInfo {
        kind,
        name: raw.printer_name.clone(),
        driver: raw.driver_name.clone(),
        port: raw.port_name.clone(),
        status,
        is_default: raw.attributes & PRINTER_ATTRIBUTE_DEFAULT != 0,
        protocol: protocol.to_string(),
        printer_class: printer_class.to_string(),
        stable_id: print_core::printer::stable_printer_id(&raw.driver_name, &raw.port_name)
            .unwrap_or_default(),
        health: Some(health),
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
    // A UNC queue (\\server\printer) is a printer shared by
    // another workstation or a print server — a normal target in
    // shops, where one printer serves several POS stations
    // through Windows sharing (external review, 2026-11-07).
    // Windows routes spooler, capabilities and DEVMODE calls for
    // a connection queue to the hosting machine, so the full
    // pipeline works for it; it is classified Shared and labelled
    // as such everywhere it is shown. Only nameless/portless
    // pseudo-printers are filtered out.
    !raw.printer_name.is_empty() && !raw.port_name.is_empty()
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

/// Process-level discovery snapshot cache with single-flight refresh.
///
/// Full spooler enumeration can take seconds when offline network
/// queues are installed, and the print worker needs a printer list
/// for every job — without this cache a 50-receipt POS burst paid a
/// full enumeration per receipt. Callers within `ttl` share one
/// snapshot; the lock is held across a refresh so concurrent misses
/// collapse into a single enumeration.
pub struct DiscoveryCache {
    snapshot: std::sync::Mutex<Option<(std::time::Instant, Vec<print_core::PrinterInfo>)>>,
}

impl DiscoveryCache {
    pub const fn new() -> Self {
        Self {
            snapshot: std::sync::Mutex::new(None),
        }
    }

    pub fn get_or_discover<E>(
        &self,
        ttl: std::time::Duration,
        discover: impl FnOnce() -> Result<Vec<print_core::PrinterInfo>, E>,
    ) -> Result<Vec<print_core::PrinterInfo>, E> {
        let mut guard = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, printers)) = &*guard {
            if at.elapsed() < ttl {
                return Ok(printers.clone());
            }
        }
        let printers = discover()?;
        *guard = Some((std::time::Instant::now(), printers.clone()));
        Ok(printers)
    }
}

impl Default for DiscoveryCache {
    fn default() -> Self {
        Self::new()
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
    fn discovery_cache_shares_snapshot_within_ttl() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = DiscoveryCache::new();
        let calls = AtomicUsize::new(0);
        let make = |cache: &DiscoveryCache, calls: &AtomicUsize| {
            cache.get_or_discover(std::time::Duration::from_secs(60), || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>(vec![print_core::PrinterInfo {
                    name: "P".into(),
                    driver: "D".into(),
                    port: "USB001".into(),
                    kind: print_core::PrinterKind::Local,
                    status: print_core::PrinterStatus::Ready,
                    is_default: true,
                    protocol: "gdi".into(),
                    printer_class: "office".into(),
                    stable_id: String::new(),
                    health: None,
                }])
            })
        };
        let a = make(&cache, &calls).unwrap();
        let b = make(&cache, &calls).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second call must hit the cache"
        );
        // An expired TTL refreshes.
        let c = cache
            .get_or_discover(std::time::Duration::ZERO, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>(Vec::new())
            })
            .unwrap();
        assert!(c.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn convert_populates_health_and_stable_id() {
        // Simultaneous flags survive as separate conditions, and the
        // endpoint identity survives a queue rename (same driver+port).
        let mut r = raw(0x0040_0000 | 0x10, 0);
        r.port_name = "USB001".into();
        let info = convert(&r);
        let health = info.health.as_ref().unwrap();
        assert!(health.has(print_core::HealthCondition::CoverOpen));
        assert!(health.has(print_core::HealthCondition::PaperOut));
        assert_eq!(info.status, health.primary_status());
        assert!(!info.stable_id.is_empty());
        let mut renamed = r.clone();
        renamed.printer_name = "Renamed Queue".into();
        assert_eq!(convert(&renamed).stable_id, info.stable_id);
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
    fn keeps_remote_unc_shared_queues_and_local_network_queues() {
        // Windows-shared printers are first-class targets
        // (2026-11-07): the filter keeps them, and classify marks
        // them Shared so every surface can label them.
        let mut remote = raw(0, 0);
        remote.printer_name = r"\\print-server\receipt".into();
        remote.port_name = "Ne03:".into();
        assert!(is_usable(&remote));
        assert_eq!(
            PrinterInfo::classify(&remote.printer_name, &remote.port_name),
            PrinterKind::Shared
        );

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

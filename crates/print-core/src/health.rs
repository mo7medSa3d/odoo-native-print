//! Printer health: a multi-condition, source-aware observation model.
//!
//! The spooler status was historically collapsed into a single
//! priority-ordered [`PrinterStatus`], which loses information: a
//! printer can be online and busy *while also* reporting low paper,
//! and a stale `Ready` from minutes ago is not evidence the device
//! is ready now. This model keeps every condition a source actually
//! reported, where it came from, when it was observed, and how long
//! the observation may be trusted. It never invents a sensor:
//! conditions only exist when a source reported them, and an
//! observation past its TTL is stale, not ready.
//!
//! [`PrinterStatus`] is kept as the compact primary status
//! ([`PrinterHealth::primary_status`]) so existing consumers and the
//! JavaScript contract keep working; the full observation rides
//! alongside it on [`PrinterInfo::health`].

use crate::printer::PrinterStatus;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One physical / queue condition. Multiple conditions coexist: a
/// printer can be [`HealthCondition::Printing`] and
/// [`HealthCondition::PaperLow`] at the same time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthCondition {
    PaperOut,
    PaperLow,
    PaperJam,
    CoverOpen,
    HeadOpen,
    RibbonOut,
    RibbonLow,
    TonerLow,
    TonerEmpty,
    InkLow,
    InkEmpty,
    /// A marker/toner/ink supply the device reports only generically
    /// (IPP `marker-supply-empty`): empty, but the supply type is
    /// not distinguished by the source.
    MarkerSupplyEmpty,
    MarkerSupplyLow,
    Paused,
    Offline,
    /// IPP printer-state 5 (stopped) or not accepting jobs: the
    /// device is present but will not take work.
    Stopped,
    Busy,
    Printing,
    WarmingUp,
    Overheating,
    HardwareError,
    CommunicationError,
    UserIntervention,
    OtherError,
    Unsupported,
    Unknown,
}

impl HealthCondition {
    /// Whether this condition, on its own, means a new job must not
    /// be submitted to the printer. Warnings (low supplies) and pure
    /// activity (busy/printing) do not block.
    pub fn blocks_printing(&self) -> bool {
        matches!(
            self,
            HealthCondition::PaperOut
                | HealthCondition::PaperJam
                | HealthCondition::CoverOpen
                | HealthCondition::HeadOpen
                | HealthCondition::RibbonOut
                | HealthCondition::TonerEmpty
                | HealthCondition::InkEmpty
                | HealthCondition::MarkerSupplyEmpty
                | HealthCondition::Paused
                | HealthCondition::Offline
                | HealthCondition::Stopped
                | HealthCondition::HardwareError
                | HealthCondition::CommunicationError
                | HealthCondition::UserIntervention
                | HealthCondition::OtherError
        )
    }
}

/// Where an observation came from. Driver-reported, firmware, and
/// protocol capabilities are different evidence and must not be
/// conflated (see the capability model in `printer.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthSource {
    /// Windows spooler / driver status flags (`PRINTER_STATUS_*`).
    Spooler,
    /// IPP `printer-state` / `printer-state-reasons` (RFC 8011).
    Ipp,
    /// ESC/POS `DLE EOT` status bytes.
    EscPos,
    /// TSPL `ESC !?` status byte.
    Tspl,
    /// ZPL `~HS` / SGD `device.host_status` response.
    Zpl,
    /// Operator-configured expectation, not a live sensor read.
    Configured,
    Unknown,
}

/// One health observation for one printer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterHealth {
    /// Every condition the source reported, sorted and deduplicated.
    /// Empty means the source reported no problem (ready).
    #[serde(default)]
    pub conditions: Vec<HealthCondition>,
    pub source: HealthSource,
    /// Unix seconds when the observation was taken.
    pub observed_at: i64,
    /// Seconds the observation may be presented as live. Past this,
    /// it is stale and must not be shown as current status.
    #[serde(default = "default_health_ttl")]
    pub ttl_secs: u64,
    /// Raw source payload (flags, reason list, status byte) kept for
    /// diagnostics. Never contains document content.
    #[serde(default)]
    pub raw: Option<String>,
}

fn default_health_ttl() -> u64 {
    30
}

impl PrinterHealth {
    pub fn new(
        conditions: BTreeSet<HealthCondition>,
        source: HealthSource,
        observed_at: i64,
        ttl_secs: u64,
    ) -> Self {
        Self {
            conditions: conditions.into_iter().collect(),
            source,
            observed_at,
            ttl_secs,
            raw: None,
        }
    }

    pub fn with_raw(mut self, raw: String) -> Self {
        self.raw = Some(raw);
        self
    }

    pub fn has(&self, condition: HealthCondition) -> bool {
        self.conditions.contains(&condition)
    }

    /// True once the observation is older than its TTL. A stale
    /// observation must never be presented as live status.
    pub fn is_stale(&self, now_unix: i64) -> bool {
        // TTL 0 means "no freshness guarantee" (the `unknown`
        // observation): it is stale at every instant, including
        // before its own timestamp, so it can never be displayed
        // as a live reading.
        self.ttl_secs == 0 || now_unix.saturating_sub(self.observed_at) > self.ttl_secs as i64
    }

    pub fn blocks_printing(&self) -> bool {
        self.conditions.iter().any(|c| c.blocks_printing())
    }

    /// The single primary status, for consumers that need one value.
    /// Priority mirrors [`crate::printer::decode_printer_status`]:
    /// what a human must fix first, then activity states. Warnings
    /// alone (low supplies) map to `Ready`; an empty condition set
    /// is `Ready`; a `CommunicationError` maps to `Unknown`, never
    /// to `Offline`, because reachability failure is not evidence
    /// the device itself is offline.
    pub fn primary_status(&self) -> PrinterStatus {
        let has = |c: HealthCondition| self.conditions.contains(&c);
        if has(HealthCondition::CoverOpen) || has(HealthCondition::HeadOpen) {
            PrinterStatus::DoorOpen
        } else if has(HealthCondition::PaperJam) {
            PrinterStatus::PaperJam
        } else if has(HealthCondition::PaperOut) {
            PrinterStatus::PaperOut
        } else if has(HealthCondition::TonerEmpty)
            || has(HealthCondition::InkEmpty)
            || has(HealthCondition::MarkerSupplyEmpty)
        {
            PrinterStatus::NoToner
        } else if has(HealthCondition::Offline) || has(HealthCondition::Stopped) {
            PrinterStatus::Offline
        } else if has(HealthCondition::UserIntervention) {
            PrinterStatus::UserIntervention
        } else if has(HealthCondition::Paused) {
            PrinterStatus::Paused
        } else if has(HealthCondition::HardwareError) || has(HealthCondition::OtherError) {
            PrinterStatus::Error
        } else if has(HealthCondition::Printing)
            || has(HealthCondition::Busy)
            || has(HealthCondition::WarmingUp)
        {
            PrinterStatus::Printing
        } else if has(HealthCondition::CommunicationError)
            || has(HealthCondition::Unknown)
            || has(HealthCondition::Unsupported)
        {
            PrinterStatus::Unknown
        } else {
            PrinterStatus::Ready
        }
    }

    /// Decode every condition in a Win32 `PRINTER_STATUS_*` flag word.
    /// Unlike the single-status decoder this preserves simultaneous
    /// conditions (door open *and* paper out). `PRINTER_STATUS_TONER_LOW`
    /// (0x20000) is read from the same word. The `PAPER_PROBLEM` flag
    /// is generic driver trouble needing attention, so it maps to
    /// `UserIntervention`, not to a fabricated specific fault.
    pub fn from_spooler_flags(flags: u32, work_offline: bool, observed_at: i64) -> Self {
        let mut set = BTreeSet::new();
        if flags & 0x0040_0000 != 0 {
            set.insert(HealthCondition::CoverOpen);
        }
        if flags & 0x0000_0008 != 0 {
            set.insert(HealthCondition::PaperJam);
        }
        if flags & 0x0000_0010 != 0 {
            set.insert(HealthCondition::PaperOut);
        }
        if flags & 0x0004_0000 != 0 {
            set.insert(HealthCondition::TonerEmpty);
        }
        if flags & 0x0000_0080 != 0 || flags & 0x0000_1000 != 0 || work_offline {
            set.insert(HealthCondition::Offline);
        }
        if flags & 0x0010_0000 != 0 || flags & 0x0000_0020 != 0 || flags & 0x0000_0800 != 0 {
            set.insert(HealthCondition::UserIntervention);
        }
        if flags & 0x0000_0040 != 0 {
            set.insert(HealthCondition::UserIntervention);
        }
        if flags & 0x0000_0001 != 0 {
            set.insert(HealthCondition::Paused);
        }
        if flags & 0x0000_0002 != 0 {
            set.insert(HealthCondition::HardwareError);
        }
        if flags & 0x0000_0400 != 0 || flags & 0x0000_4000 != 0 {
            set.insert(HealthCondition::Printing);
        }
        if flags & 0x0000_0200 != 0 {
            set.insert(HealthCondition::Busy);
        }
        if flags & 0x0002_0000 != 0 {
            set.insert(HealthCondition::TonerLow);
        }
        Self::new(set, HealthSource::Spooler, observed_at, 30)
            .with_raw(format!("spooler_flags=0x{flags:08x}"))
    }

    /// Build from IPP evidence without depending on the IPP crate:
    /// `state` is `printer-state` (3 idle, 4 processing, 5 stopped),
    /// `reasons` are `printer-state-reasons` values with RFC 8011
    /// severity suffixes already stripped by the caller, and
    /// `accepting_jobs` is `printer-is-accepting-jobs` (`None` =
    /// not reported, unknown, not assumed).
    pub fn from_ipp(
        state: i32,
        reasons: &[&str],
        accepting_jobs: Option<bool>,
        observed_at: i64,
    ) -> Self {
        let mut set = BTreeSet::new();
        for reason in reasons {
            match *reason {
                "media-empty" => {
                    set.insert(HealthCondition::PaperOut);
                }
                "media-low" => {
                    set.insert(HealthCondition::PaperLow);
                }
                "media-jam" => {
                    set.insert(HealthCondition::PaperJam);
                }
                "door-open" | "cover-open" => {
                    set.insert(HealthCondition::CoverOpen);
                }
                "marker-supply-empty" => {
                    set.insert(HealthCondition::MarkerSupplyEmpty);
                }
                "marker-supply-low" | "toner-low" | "marker-low" => {
                    set.insert(HealthCondition::TonerLow);
                }
                "toner-empty" => {
                    set.insert(HealthCondition::TonerEmpty);
                }
                "offline" => {
                    set.insert(HealthCondition::Offline);
                }
                "paused" => {
                    set.insert(HealthCondition::Paused);
                }
                "input-tray-missing" | "output-full" | "intervention-needed" => {
                    set.insert(HealthCondition::UserIntervention);
                }
                "media-needed" | "media-ready" | "none" => {}
                _ => {
                    set.insert(HealthCondition::OtherError);
                }
            }
        }
        match state {
            4 => {
                set.insert(HealthCondition::Printing);
            }
            5 => {
                set.insert(HealthCondition::Stopped);
            }
            _ => {}
        }
        if accepting_jobs == Some(false) {
            set.insert(HealthCondition::Stopped);
        }
        Self::new(set, HealthSource::Ipp, observed_at, 30)
    }

    /// TSPL `ESC !?` one-byte status (TSC TSPL/TSPL2 Programming
    /// Manual; Zebra TSPL guide value table): bit 0 head opened,
    /// bit 1 paper jam, bit 2 out of paper, bit 3 out of ribbon,
    /// bit 4 pause, bit 5 printing, bit 6 cover opened (option,
    /// model-dependent), bit 7 other error. 0x00 is ready.
    pub fn from_tspl_status(byte: u8, observed_at: i64) -> Self {
        let mut set = BTreeSet::new();
        if byte & 0x01 != 0 {
            set.insert(HealthCondition::HeadOpen);
        }
        if byte & 0x02 != 0 {
            set.insert(HealthCondition::PaperJam);
        }
        if byte & 0x04 != 0 {
            set.insert(HealthCondition::PaperOut);
        }
        if byte & 0x08 != 0 {
            set.insert(HealthCondition::RibbonOut);
        }
        if byte & 0x10 != 0 {
            set.insert(HealthCondition::Paused);
        }
        if byte & 0x20 != 0 {
            set.insert(HealthCondition::Printing);
        }
        if byte & 0x40 != 0 {
            set.insert(HealthCondition::CoverOpen);
        }
        if byte & 0x80 != 0 {
            set.insert(HealthCondition::OtherError);
        }
        Self::new(set, HealthSource::Tspl, observed_at, 30)
            .with_raw(format!("tspl_status=0x{byte:02x}"))
    }

    /// ZPL `~HS` / SGD host-status flags, already parsed positionally
    /// by the ZPL parser in print-raw. Buffer-full and label-waiting
    /// are informational activity, not faults, and add no condition.
    #[allow(clippy::too_many_arguments)]
    pub fn from_zpl_flags(
        paper_out: bool,
        paused: bool,
        head_up: bool,
        ribbon_out: bool,
        buffer_full: bool,
        observed_at: i64,
    ) -> Self {
        let mut set = BTreeSet::new();
        if paper_out {
            set.insert(HealthCondition::PaperOut);
        }
        if paused {
            set.insert(HealthCondition::Paused);
        }
        if head_up {
            set.insert(HealthCondition::HeadOpen);
        }
        if ribbon_out {
            set.insert(HealthCondition::RibbonOut);
        }
        if buffer_full {
            set.insert(HealthCondition::Busy);
        }
        Self::new(set, HealthSource::Zpl, observed_at, 30)
    }

    /// ESC/POS `DLE EOT` bytes (n=1 printer, n=2 offline-cause,
    /// n=3 error-cause, n=4 paper sensors). Any frame may be absent
    /// (`None`) when the transport could not obtain it; missing
    /// evidence adds no condition. Bit assignments follow the Epson
    /// TM reference (see print-raw `tcp9100`): n=1/n=2 bit 3 offline,
    /// bit 6 error; n=3 bit 2 mechanical, bit 3 cutter, bit 5
    /// unrecoverable, bit 6 auto-recoverable (e.g. head overheat);
    /// n=4 bits 2-3 near-end pair, bits 5-6 paper-end pair.
    pub fn from_escpos(
        n1: Option<u8>,
        n2: Option<u8>,
        n3: Option<u8>,
        n4: Option<u8>,
        observed_at: i64,
    ) -> Self {
        let mut set = BTreeSet::new();
        for frame in [n1, n2].into_iter().flatten() {
            if frame & 0x08 != 0 {
                set.insert(HealthCondition::Offline);
            }
            if frame & 0x40 != 0 {
                set.insert(HealthCondition::HardwareError);
            }
        }
        if let Some(b) = n3 {
            if b & 0x04 != 0 || b & 0x08 != 0 || b & 0x20 != 0 {
                set.insert(HealthCondition::HardwareError);
            }
            if b & 0x40 != 0 {
                set.insert(HealthCondition::Overheating);
            }
        }
        if let Some(b) = n4 {
            if (b >> 2) & 0x03 == 0x03 {
                set.insert(HealthCondition::PaperLow);
            }
            if (b >> 5) & 0x03 == 0x03 {
                set.insert(HealthCondition::PaperOut);
            }
        }
        Self::new(set, HealthSource::EscPos, observed_at, 30)
    }

    /// An explicit "no live evidence" observation: unknown, and
    /// stale immediately (TTL 0) so it can never be displayed as a
    /// current ready state.
    pub fn unknown(source: HealthSource, observed_at: i64) -> Self {
        let mut set = BTreeSet::new();
        set.insert(HealthCondition::Unknown);
        Self::new(set, source, observed_at, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spooler_flags_preserve_simultaneous_conditions() {
        // Door open AND paper out AND toner low: the old single-status
        // decoder kept only the door; the health model keeps all three.
        let h = PrinterHealth::from_spooler_flags(0x0040_0010 | 0x0002_0000, false, 1000);
        assert!(h.has(HealthCondition::CoverOpen));
        assert!(h.has(HealthCondition::PaperOut));
        assert!(h.has(HealthCondition::TonerLow));
        assert!(h.blocks_printing());
        assert_eq!(h.primary_status(), PrinterStatus::DoorOpen);
        // A warning alone does not block and reads as ready.
        let warn = PrinterHealth::from_spooler_flags(0x0002_0000, false, 1000);
        assert!(!warn.blocks_printing());
        assert_eq!(warn.primary_status(), PrinterStatus::Ready);
        assert_eq!(warn.conditions, vec![HealthCondition::TonerLow]);
    }

    #[test]
    fn spooler_zero_flags_is_ready_and_work_offline_is_offline() {
        let h = PrinterHealth::from_spooler_flags(0, false, 1000);
        assert!(h.conditions.is_empty());
        assert_eq!(h.primary_status(), PrinterStatus::Ready);
        let off = PrinterHealth::from_spooler_flags(0, true, 1000);
        assert!(off.has(HealthCondition::Offline));
        assert!(off.blocks_printing());
    }

    #[test]
    fn ipp_reasons_keep_warnings_distinct_from_blockers() {
        let h = PrinterHealth::from_ipp(3, &["media-low", "toner-low"], Some(true), 1000);
        assert!(h.has(HealthCondition::PaperLow));
        assert!(!h.blocks_printing());
        let blocked = PrinterHealth::from_ipp(5, &["media-empty", "door-open"], Some(false), 1000);
        assert!(blocked.blocks_printing());
        assert_eq!(blocked.primary_status(), PrinterStatus::DoorOpen);
        // Unknown reason: recorded as OtherError, never dropped.
        let other = PrinterHealth::from_ipp(3, &["fuser-overtemp"], Some(true), 1000);
        assert!(other.has(HealthCondition::OtherError));
    }

    #[test]
    fn tspl_byte_table() {
        // Values from the Zebra TSPL guide table.
        assert!(PrinterHealth::from_tspl_status(0x00, 0)
            .conditions
            .is_empty());
        let h = PrinterHealth::from_tspl_status(0x0D, 0);
        assert!(h.has(HealthCondition::HeadOpen));
        assert!(h.has(HealthCondition::PaperOut));
        assert!(h.has(HealthCondition::RibbonOut));
        let h = PrinterHealth::from_tspl_status(0x80, 0);
        assert!(h.has(HealthCondition::OtherError));
        assert!(h.blocks_printing());
        let h = PrinterHealth::from_tspl_status(0x20, 0);
        assert!(h.has(HealthCondition::Printing));
        assert!(!h.blocks_printing());
    }

    #[test]
    fn escpos_frames_combine_without_inventing_missing_evidence() {
        let h = PrinterHealth::from_escpos(None, Some(0x12 | 0x08), None, Some(0x12 | 0x0C), 0);
        assert!(h.has(HealthCondition::Offline));
        assert!(h.has(HealthCondition::PaperLow));
        assert!(!h.has(HealthCondition::PaperOut));
        let none = PrinterHealth::from_escpos(None, None, None, None, 0);
        assert!(none.conditions.is_empty());
        let err = PrinterHealth::from_escpos(None, None, Some(0x12 | 0x20), None, 0);
        assert!(err.has(HealthCondition::HardwareError));
        let hot = PrinterHealth::from_escpos(None, None, Some(0x12 | 0x40), None, 0);
        assert!(hot.has(HealthCondition::Overheating));
    }

    #[test]
    fn staleness_and_unknown_observation() {
        let h = PrinterHealth::from_spooler_flags(0, false, 1000);
        assert!(!h.is_stale(1029));
        assert!(!h.is_stale(1030));
        assert!(h.is_stale(1031));
        let u = PrinterHealth::unknown(HealthSource::Unknown, 1000);
        assert!(u.is_stale(1000));
        assert_eq!(u.primary_status(), PrinterStatus::Unknown);
        assert!(!u.blocks_printing());
    }

    #[test]
    fn health_serializes_with_snake_case_conditions() {
        let h = PrinterHealth::from_tspl_status(0x04, 1000);
        let json = serde_json::to_string(&h).unwrap();
        assert!(json.contains("paper_out"));
        let back: PrinterHealth = serde_json::from_str(&json).unwrap();
        assert_eq!(back, h);
    }
}

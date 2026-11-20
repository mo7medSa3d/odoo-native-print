//! Job routing helpers: printer-pattern matching and picking the
//! live target printer from the ones actually present on this
//! device. (The earlier server-side rule system was removed with
//! the live-printer model: printing resolves on this PC at click
//! time; the server keeps only the job log.)

use crate::printer::{needs_intervention, PrinterInfo, PrinterStatus};

/// Printer pattern matching: `*` is the only wildcard (any run of
/// characters). Without `*` the match is exact. Case-insensitive.
/// Examples: "HP LaserJet" exact; "HP*" prefix; "*thermal*" contains.
pub fn printer_pattern_matches(pattern: &str, printer_name: &str) -> bool {
    let p_trim = pattern.trim();
    let n_trim = printer_name.trim();
    if !p_trim.contains('*') {
        if p_trim.is_ascii() && n_trim.is_ascii() {
            return p_trim.eq_ignore_ascii_case(n_trim);
        }
        return p_trim.to_lowercase() == n_trim.to_lowercase();
    }
    let p = p_trim.to_lowercase();
    let n = n_trim.to_lowercase();
    // Glob with * segments: match segments in order.
    let mut pos = 0usize;
    let mut last_seg = "";
    for (i, seg) in p.split('*').enumerate() {
        last_seg = seg;
        if seg.is_empty() {
            continue;
        }
        match n[pos..].find(seg) {
            Some(off) => {
                if i == 0 && off != 0 {
                    return false; // first segment must anchor at the start
                }
                pos += off + seg.len();
            }
            None => return false,
        }
    }
    // A trailing segment (after the last *) must anchor at the end.
    last_seg.is_empty() || n.ends_with(last_seg)
}

/// Pick the best printer satisfying a pattern. Healthy printers win over
/// printers that need intervention, and within a tier the OS default
/// printer wins over non-default printers; among equals, the first wins.
pub fn resolve_printer<'a>(pattern: &str, printers: &'a [PrinterInfo]) -> Option<&'a PrinterInfo> {
    let mut best: Option<(u8, &'a PrinterInfo)> = None;
    for p in printers {
        if !printer_pattern_matches(pattern, &p.name) {
            continue;
        }
        let healthy = p.status == PrinterStatus::Ready || p.status == PrinterStatus::Printing;
        let rank = if healthy && p.is_default {
            return Some(p);
        } else if healthy {
            1
        } else if !needs_intervention(p.status) && p.is_default {
            2
        } else if !needs_intervention(p.status) {
            3
        } else if p.is_default {
            4
        } else {
            5
        };
        if best.is_none_or(|(best_rank, _)| rank < best_rank) {
            best = Some((rank, p));
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::PrinterKind;

    fn printer(name: &str, status: PrinterStatus) -> PrinterInfo {
        PrinterInfo {
            name: name.into(),
            driver: "d".into(),
            port: "USB001".into(),
            kind: PrinterKind::Local,
            status,
            is_default: false,
            protocol: "gdi".into(),
            printer_class: "office".into(),
            stable_id: String::new(),
            health: None,
        }
    }

    #[test]
    fn pattern_matching() {
        assert!(printer_pattern_matches("HP LaserJet", "hp laserjet"));
        assert!(printer_pattern_matches("  HP LaserJet ", "HP LaserJet"));
        assert!(printer_pattern_matches("HP*", "HP LaserJet Pro"));
        assert!(!printer_pattern_matches("HP*", "Canon HP clone"));
        assert!(printer_pattern_matches("*thermal*", "EPSON Thermal 80"));
        assert!(printer_pattern_matches("EPSON*T88", "EPSON TM-T88"));
        assert!(printer_pattern_matches("EPSON*T88*", "EPSON TM-T88VII"));
        assert!(!printer_pattern_matches("EPSON*T88", "EPSON TM-T88VII"));
        assert!(!printer_pattern_matches("EPSON*T88", "EPSON TM-T20"));
        assert!(!printer_pattern_matches("Zebra", "Zebra ZD230 extra"));
    }

    #[test]
    fn printer_choice_prefers_healthy() {
        let printers = vec![
            printer("HP A", PrinterStatus::PaperOut),
            printer("HP B", PrinterStatus::Ready),
            printer("Canon", PrinterStatus::Ready),
        ];
        let chosen = resolve_printer("HP*", &printers).unwrap();
        assert_eq!(chosen.name, "HP B");
        // When every match is unhealthy, still return one (the worker's
        // job will fail with that printer's status, which is the truth
        // the user needs) rather than silently using another printer.
        let sick = vec![printer("HP A", PrinterStatus::Offline)];
        assert_eq!(resolve_printer("HP*", &sick).unwrap().name, "HP A");
        assert!(resolve_printer("Zebra*", &printers).is_none());
    }

    #[test]
    fn resolve_printer_prefers_default_among_healthy() {
        let mut p2 = printer("EPSON TM-T88", PrinterStatus::Ready);
        p2.is_default = true;
        let printers = vec![printer("HP LaserJet", PrinterStatus::Ready), p2];
        let chosen = resolve_printer("*", &printers).unwrap();
        assert_eq!(chosen.name, "EPSON TM-T88");
    }

    #[test]
    fn resolve_shared_unc_printer_prefers_healthy() {
        let mut offline_shared = printer(r"\\POS-1\XP-80C", PrinterStatus::Offline);
        offline_shared.kind = PrinterKind::Shared;
        let mut ready_shared = printer(r"\\POS-2\XP-80C", PrinterStatus::Ready);
        ready_shared.kind = PrinterKind::Shared;
        let printers = vec![offline_shared, ready_shared];
        assert!(printer_pattern_matches(
            r"\\POS-2\XP-80C",
            r"\\pos-2\xp-80c"
        ));
        let chosen = resolve_printer(r"\\*\XP-80C", &printers).unwrap();
        assert_eq!(chosen.name, r"\\POS-2\XP-80C");
    }

    #[test]
    fn resolve_printer_respects_unknown_default_over_offline() {
        let mut unk_default = printer("HP Unknown Default", PrinterStatus::Unknown);
        unk_default.is_default = true;
        let printers = vec![
            printer("HP Offline", PrinterStatus::Offline),
            printer("HP Unknown", PrinterStatus::Unknown),
            unk_default,
        ];
        let chosen = resolve_printer("HP*", &printers).unwrap();
        assert_eq!(chosen.name, "HP Unknown Default");
        let mut off_default = printer("HP Offline Default", PrinterStatus::Offline);
        off_default.is_default = true;
        let sick = vec![printer("HP PaperOut", PrinterStatus::PaperOut), off_default];
        assert_eq!(
            resolve_printer("HP*", &sick).unwrap().name,
            "HP Offline Default"
        );
    }
}

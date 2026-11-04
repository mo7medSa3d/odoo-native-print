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
    let p = pattern.to_lowercase();
    let n = printer_name.to_lowercase();
    if !p.contains('*') {
        return p == n;
    }
    // Glob with * segments: match segments in order.
    let segments: Vec<&str> = p.split('*').collect();
    let mut pos = 0usize;
    for (i, seg) in segments.iter().enumerate() {
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
    if let Some(last) = segments.last() {
        if !last.is_empty() && !n.ends_with(last) {
            return false;
        }
    }
    true
}

/// Pick the best printer satisfying a pattern. Healthy printers win over
/// printers that need intervention; among equals, the first wins (the
/// enumeration order is the OS's, which puts the default early in
/// practice but nothing here depends on that).
pub fn resolve_printer<'a>(pattern: &str, printers: &'a [PrinterInfo]) -> Option<&'a PrinterInfo> {
    let matches: Vec<&PrinterInfo> = printers
        .iter()
        .filter(|p| printer_pattern_matches(pattern, &p.name))
        .collect();
    if let Some(p) = matches
        .iter()
        .find(|p| p.status == PrinterStatus::Ready || p.status == PrinterStatus::Printing)
    {
        return Some(p);
    }
    if let Some(p) = matches.iter().find(|p| !needs_intervention(p.status)) {
        return Some(p);
    }
    matches.first().copied()
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
}

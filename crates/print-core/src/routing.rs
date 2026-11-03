//! Job routing: given the server's rules and the printers actually
//! present on this device, decide where a job goes. This mirrors the
//! addon's server-side resolution (the server is the authority); the
//! client-side copy exists so the app can also route locally queued
//! jobs (attachments, reprints) with the same semantics.

use crate::printer::{needs_intervention, PrinterInfo, PrinterStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintRule {
    /// None means "any report".
    pub report_name: Option<String>,
    /// None means "any user".
    pub group: Option<String>,
    pub company: String,
    pub profile_name: String,
    pub printer_pattern: String,
    pub priority: i64,
}

/// Resolve the winning rule with the same semantics as the server
/// resolver (the server is the authority): an exact report match
/// outranks the catch-all regardless of priority; priority decides
/// within a tier; ties keep the earlier rule (stable). Company must
/// match; a group-scoped rule applies only to group members.
pub fn resolve_rule<'a>(
    rules: &'a [PrintRule],
    report_name: &str,
    user_groups: &[String],
    company: &str,
) -> Option<&'a PrintRule> {
    rules
        .iter()
        .filter(|r| r.company == company)
        .filter(|r| r.report_name.as_deref().is_none_or(|n| n == report_name))
        .filter(|r| {
            r.group
                .as_deref()
                .is_none_or(|g| user_groups.iter().any(|u| u == g))
        })
        .enumerate()
        .max_by(|(ia, a), (ib, b)| {
            let tier = |r: &PrintRule| if r.report_name.is_some() { 1 } else { 0 };
            tier(a)
                .cmp(&tier(b))
                .then(a.priority.cmp(&b.priority))
                // Stable ties: the earlier rule wins, so on equal keys
                // the element with the smaller index must compare
                // greater for max_by to keep it.
                .then(ib.cmp(ia))
        })
        .map(|(_, r)| r)
}

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

    fn rule(report: Option<&str>, group: Option<&str>, prio: i64, profile: &str) -> PrintRule {
        PrintRule {
            report_name: report.map(|s| s.to_string()),
            group: group.map(|s| s.to_string()),
            company: "Acme".into(),
            profile_name: profile.into(),
            printer_pattern: "HP*".into(),
            priority: prio,
        }
    }
    fn printer(name: &str, status: PrinterStatus) -> PrinterInfo {
        PrinterInfo {
            name: name.into(),
            driver: "d".into(),
            port: "USB001".into(),
            kind: PrinterKind::Local,
            status,
            is_default: false,
        }
    }

    #[test]
    fn rule_resolution_priority_and_filters() {
        let rules = vec![
            rule(None, None, 1, "catch-all"),
            rule(Some("sale.report"), None, 5, "sales"),
            rule(Some("sale.report"), Some("sales_team"), 50, "sales-vip"),
            rule(Some("stock.label"), None, 99, "labels"),
        ];
        let groups = vec!["sales_team".to_string()];
        // VIP rule wins for a sales team member printing a sale report.
        let r = resolve_rule(&rules, "sale.report", &groups, "Acme").unwrap();
        assert_eq!(r.profile_name, "sales-vip");
        // Without the group, the plain sales rule wins.
        let r = resolve_rule(&rules, "sale.report", &[], "Acme").unwrap();
        assert_eq!(r.profile_name, "sales");
        // Unknown report falls to the catch-all; wrong company gets nothing.
        let r = resolve_rule(&rules, "account.move", &groups, "Acme").unwrap();
        assert_eq!(r.profile_name, "catch-all");
        assert!(resolve_rule(&rules, "sale.report", &groups, "Other").is_none());
    }
    #[test]
    fn exact_report_beats_catch_all_priority_and_ties_are_stable() {
        // A high-priority catch-all must not shadow an exact rule
        // (server semantics), and equal keys keep the earlier rule.
        let rules = vec![
            rule(None, None, 100, "greedy-catch-all"),
            rule(Some("sale.report"), None, 1, "exact"),
            rule(Some("sale.report"), None, 1, "exact-later"),
        ];
        let r = resolve_rule(&rules, "sale.report", &[], "Acme").unwrap();
        assert_eq!(r.profile_name, "exact");
        let r = resolve_rule(&rules, "other.report", &[], "Acme").unwrap();
        assert_eq!(r.profile_name, "greedy-catch-all");
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

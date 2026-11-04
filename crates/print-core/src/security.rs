//! Security validation that runs in Rust on every IPC command.
//! Rationale (research, 2026-11-02): Tauri app commands are not
//! permission-gated by default, and CVE-2026-42184 showed origin
//! classification itself can be wrong. So we never trust it alone.

/// Exact-origin check. `configured` is the configured Odoo origin and
/// `actual` is the caller's origin. Exact scheme, host and port matching
/// is required; credentials, paths, queries and fragments are rejected.
///
/// Scheme policy (ADR-0002, owner decision 2026-11-03): HTTPS is
/// accepted for any host. HTTP is accepted for any IP literal — LAN
/// installs are reached by IP (e.g. http://192.168.1.20:8069) and the
/// owner also accepts external IPs over HTTP. Hostnames over HTTP are
/// rejected. HTTP is unencrypted: the Control Center shows a standing
/// warning whenever an HTTP origin is configured.
pub fn origin_is_allowed(configured: &str, actual: &str) -> bool {
    let (Some(c), Some(a)) = (parse_origin(configured), parse_origin(actual)) else {
        return false;
    };
    c == a && scheme_allowed(&c)
}

/// Host (IPv6 brackets stripped) and effective port of a valid
/// origin, for reachability probes. Returns None for anything
/// `parse_origin` rejects, so probe code never re-implements the
/// security-critical parser in a second place (audit L5).
pub fn origin_host_port(origin: &str) -> Option<(String, u16)> {
    let (_scheme, host, port) = parse_origin(origin)?;
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(&host)
        .to_string();
    Some((bare, port))
}

fn scheme_allowed(origin: &(String, String, u16)) -> bool {
    origin.0 == "https" || (origin.0 == "http" && host_is_ip_literal(&origin.1))
}

/// True when `host` is a syntactically valid IPv4 or IPv6 literal
/// (private, loopback, link-local, or public). Hostnames return false.
fn host_is_ip_literal(host: &str) -> bool {
    use std::net::IpAddr;
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse::<IpAddr>().is_ok()
}

/// Minimal strict origin parser: scheme://host[:port] and nothing else.
/// Returns (scheme, host_lowercase, effective_port).
fn parse_origin(s: &str) -> Option<(String, String, u16)> {
    let (scheme, rest) = s.split_once("://")?;
    if rest.is_empty() || rest.contains(['/', '?', '#', '@', '\\']) {
        return None;
    }
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let default_port = if scheme.eq_ignore_ascii_case("https") {
        443
    } else {
        80
    };
    let (host, port) = if let Some(after_open) = rest.strip_prefix('[') {
        let (ipv6, after_close) = after_open.split_once(']')?;
        if ipv6.is_empty() {
            return None;
        }
        let port = if after_close.is_empty() {
            default_port
        } else {
            let p = after_close.strip_prefix(':')?;
            if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            p.parse::<u16>().ok()?
        };
        (format!("[{ipv6}]"), port)
    } else {
        match rest.rsplit_once(':') {
            Some((h, p)) => {
                if h.is_empty() || p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                (h.to_string(), p.parse::<u16>().ok()?)
            }
            None => (rest.to_string(), default_port),
        }
    };
    if host.is_empty() || host.contains(char::is_whitespace) || port == 0 {
        return None;
    }
    Some((
        scheme.to_ascii_lowercase(),
        canonical_host(&host.to_ascii_lowercase()),
        port,
    ))
}

/// Canonicalize a parsed host the way the webview's URL serializer
/// reports it, applied to BOTH sides of every origin comparison
/// (audit L3): an admin who types an uncompressed IPv6 literal
/// (`[0:0:0:0:0:0:0:1]`) or a trailing-dot hostname otherwise passes
/// save-time validation (self-comparison) while the webview reports
/// the canonical form (`[::1]`) — every runtime check then fails and
/// the app locks itself out with no visible reason. IP literals are
/// round-tripped through `IpAddr`'s Display; hostnames lose one
/// trailing dot.
fn canonical_host(host: &str) -> String {
    use std::net::IpAddr;
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        if let Ok(ip) = inner.parse::<IpAddr>() {
            return format!("[{ip}]");
        }
        return host.to_string();
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return ip.to_string();
    }
    host.strip_suffix('.').unwrap_or(host).to_string()
}

/// Hard limits for job input (spec sections 5 and 7).
pub const MAX_JOB_BYTES: usize = 50 * 1024 * 1024; // S4 tested size is also the cap
pub const MAX_PAGES: u32 = 2_000;
pub const MAX_COPIES: u16 = 99;
pub const MAX_DIMENSION_MM: f64 = 1_500.0;

#[derive(Debug, PartialEq, Eq)]
pub enum LimitError {
    TooLarge,
    TooManyPages,
    TooManyCopies,
    BadDimension,
    Empty,
}

/// The parse-independent part of job validation: payload size and
/// copy count. MUST run before any PDF/PNG parsing of untrusted bytes
/// so a hostile or corrupt document is rejected by size before a
/// parser ever touches it (the full check, including page count and
/// dimensions, still runs after parsing).
pub fn validate_payload_limits(bytes_len: usize, copies: u16) -> Result<(), LimitError> {
    if bytes_len == 0 {
        return Err(LimitError::Empty);
    }
    if bytes_len > MAX_JOB_BYTES {
        return Err(LimitError::TooLarge);
    }
    if copies == 0 || copies > MAX_COPIES {
        return Err(LimitError::TooManyCopies);
    }
    Ok(())
}

pub fn validate_job_input(
    bytes_len: usize,
    pages: u32,
    copies: u16,
    page: crate::PageSize,
) -> Result<(), LimitError> {
    if bytes_len == 0 {
        return Err(LimitError::Empty);
    }
    if bytes_len > MAX_JOB_BYTES {
        return Err(LimitError::TooLarge);
    }
    if pages == 0 || pages > MAX_PAGES {
        return Err(LimitError::TooManyPages);
    }
    if copies == 0 || copies > MAX_COPIES {
        return Err(LimitError::TooManyCopies);
    }
    if !page.width_mm.is_finite()
        || !page.height_mm.is_finite()
        || page.width_mm <= 0.0
        || page.height_mm <= 0.0
        || page.width_mm > MAX_DIMENSION_MM
        || page.height_mm > MAX_DIMENSION_MM
    {
        return Err(LimitError::BadDimension);
    }
    Ok(())
}

/// Idempotency keys: non-empty, bounded, printable ASCII (they are logged).
pub fn valid_idempotency_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 128 && key.chars().all(|c| c.is_ascii_graphic())
}

#[cfg(test)]
mod tests {
    use super::*;
    const CFG: &str = "http://192.168.1.20:8069";
    #[test]
    fn exact_origin_passes() {
        assert!(origin_is_allowed(CFG, CFG));
    }
    #[test]
    fn domains_and_scheme_changes_fail() {
        assert!(!origin_is_allowed(CFG, "http://odoo.evil.com:8069"));
        assert!(!origin_is_allowed(
            "http://app.localhost",
            "http://app.evil.com"
        ));
        assert!(!origin_is_allowed(CFG, "https://192.168.1.20:8069"));
    }
    #[test]
    fn http_and_port_and_userinfo_fail() {
        assert!(!origin_is_allowed(CFG, "http://odoo.company.local:8069"));
        assert!(!origin_is_allowed(CFG, "https://odoo.company.local:9999"));
        assert!(!origin_is_allowed(CFG, "https://odoo.company.local:abc"));
        assert!(!origin_is_allowed(
            CFG,
            "https://user@odoo.company.local:8069"
        ));
        assert!(!origin_is_allowed(
            CFG,
            "https://odoo.company.local:8069/path"
        ));
    }
    #[test]
    fn https_any_host_and_http_any_ip() {
        // HTTPS: any host, name or IP.
        assert!(origin_is_allowed(
            "https://odoo.company.local:8069",
            "https://odoo.company.local:8069"
        ));
        assert!(origin_is_allowed(
            "https://192.168.1.20:8069",
            "https://192.168.1.20:8069"
        ));
        assert!(origin_is_allowed(
            "https://8.8.8.8:8069",
            "https://8.8.8.8:8069"
        ));
        // HTTP: any IP literal, private or public.
        assert!(origin_is_allowed(
            "http://192.168.1.20:8069",
            "http://192.168.1.20:8069"
        ));
        assert!(origin_is_allowed("http://10.0.0.5", "http://10.0.0.5:80"));
        assert!(origin_is_allowed(
            "http://172.16.4.9:8069",
            "http://172.16.4.9:8069"
        ));
        assert!(origin_is_allowed(
            "http://127.0.0.1:8069",
            "http://127.0.0.1:8069"
        ));
        assert!(origin_is_allowed("http://[::1]:8069", "http://[::1]:8069"));
        assert!(origin_is_allowed(
            "http://[fd00::25]:8069",
            "http://[fd00::25]:8069"
        ));
        assert!(origin_is_allowed(
            "http://[fe80::1]:8069",
            "http://[fe80::1]:8069"
        ));
        assert!(origin_is_allowed(
            "http://8.8.8.8:8069",
            "http://8.8.8.8:8069"
        ));
        assert!(origin_is_allowed(
            "http://68.221.27.248:8070",
            "http://68.221.27.248:8070"
        ));
        assert!(origin_is_allowed(
            "http://172.32.0.1:8069",
            "http://172.32.0.1:8069"
        ));
        assert!(origin_is_allowed(
            "http://[2001:4860:4860::8888]:8069",
            "http://[2001:4860:4860::8888]:8069"
        ));
        // HTTP to hostnames stays rejected (names require HTTPS).
        assert!(!origin_is_allowed(
            "http://odoo.company.local:8069",
            "http://odoo.company.local:8069"
        ));
        // Scheme must match exactly between configured and actual.
        assert!(!origin_is_allowed(
            "https://odoo.company.local:8069",
            "http://odoo.company.local:8069"
        ));
    }
    #[test]
    fn default_port_and_ipv6_equivalence() {
        assert!(origin_is_allowed(
            "http://192.168.1.20",
            "http://192.168.1.20:80"
        ));
        assert!(origin_is_allowed("http://[::1]:8069", "http://[::1]:8069"));
    }
    #[test]
    fn noncanonical_forms_match_the_webviews_serialization() {
        // Hand-typed uncompressed IPv6 vs the webview's compressed
        // serialization (audit L3): both sides canonicalize.
        assert!(origin_is_allowed(
            "http://[0:0:0:0:0:0:0:1]:8069",
            "http://[::1]:8069"
        ));
        assert!(origin_is_allowed(
            "http://[fd00:0:0:0:0:0:0:25]:8069",
            "http://[fd00::25]:8069"
        ));
        // Trailing-dot hostname (HTTPS) vs the bare form.
        assert!(origin_is_allowed(
            "https://odoo.company.local.:8069",
            "https://odoo.company.local:8069"
        ));
        // Canonicalization never widens the match to a different host.
        assert!(!origin_is_allowed(
            "http://[0:0:0:0:0:0:0:2]:8069",
            "http://[::1]:8069"
        ));
    }
    #[test]
    fn host_port_extraction_uses_the_same_parser() {
        assert_eq!(
            origin_host_port("http://192.168.1.20:8069"),
            Some(("192.168.1.20".to_string(), 8069))
        );
        assert_eq!(
            origin_host_port("http://[0:0:0:0:0:0:0:1]:8069"),
            Some(("::1".to_string(), 8069))
        );
        assert_eq!(
            origin_host_port("https://odoo.company.local"),
            Some(("odoo.company.local".to_string(), 443))
        );
        assert_eq!(origin_host_port("http://host/path"), None);
        assert_eq!(origin_host_port("not a url"), None);
    }
    #[test]
    fn limits() {
        let a4 = crate::PageSize::a4();
        assert!(validate_job_input(1024, 1, 1, a4).is_ok());
        assert_eq!(
            validate_job_input(MAX_JOB_BYTES + 1, 1, 1, a4),
            Err(LimitError::TooLarge)
        );
        assert_eq!(
            validate_job_input(10, 0, 1, a4),
            Err(LimitError::TooManyPages)
        );
        assert_eq!(
            validate_job_input(10, 1, 1, crate::PageSize::new(f64::NAN, 297.0)),
            Err(LimitError::BadDimension)
        );
        assert!(valid_idempotency_key("job-123_abc"));
        assert!(!valid_idempotency_key(""));
        assert!(!valid_idempotency_key("has space"));
    }
}

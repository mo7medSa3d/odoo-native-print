//! Security validation that runs in Rust on every IPC command.
//! Rationale (research, 2026-11-02): Tauri app commands are not
//! permission-gated by default, and CVE-2026-42184 showed origin
//! classification itself can be wrong. So we never trust it alone.

/// Exact-origin check. `configured` is the configured Odoo origin and
/// `actual` is the caller's origin. Exact scheme, host and port matching
/// is required; credentials, paths, queries and fragments are rejected.
///
/// Scheme policy (ADR-0002, owner decision 2026-11-03): HTTPS is
/// accepted for any host. HTTP is accepted for `localhost` (exact
/// loopback hostname only, never subdomains) and for any IP literal —
/// LAN installs are reached by IP (e.g. http://192.168.1.20:8069) and
/// the owner also accepts external IPs over HTTP. Other hostnames over
/// HTTP are rejected.
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

fn scheme_allowed(origin: &(&'static str, String, u16)) -> bool {
    origin.0 == "https"
        || (origin.0 == "http" && (origin.1 == "localhost" || host_is_ip_literal(&origin.1)))
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
/// Returns (scheme, canonical_host_lowercase, effective_port).
/// Canonicalizes IP literals via `Display` and strips one trailing dot
/// from hostnames so admin-typed origins match webview URL serialization
/// (audit L3).
fn parse_origin(s: &str) -> Option<(&'static str, String, u16)> {
    let (scheme, rest) = s.split_once("://")?;
    if rest.is_empty()
        || rest.contains(['/', '?', '#', '@', '\\'])
        || rest.chars().any(char::is_control)
    {
        return None;
    }
    let (scheme_norm, default_port) = if scheme.eq_ignore_ascii_case("https") {
        ("https", 443)
    } else if scheme.eq_ignore_ascii_case("http") {
        ("http", 80)
    } else {
        return None;
    };
    if let Some(after_open) = rest.strip_prefix('[') {
        let (ipv6, after_close) = after_open.split_once(']')?;
        let ip: std::net::Ipv6Addr = ipv6.parse().ok()?;
        let port = if after_close.is_empty() {
            default_port
        } else {
            let p = after_close.strip_prefix(':')?;
            if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            p.parse::<u16>().ok()?
        };
        if port == 0 {
            return None;
        }
        return Some((scheme_norm, format!("[{ip}]"), port));
    }
    let (raw_host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => {
            if h.is_empty()
                || h.contains(':')
                || p.is_empty()
                || !p.chars().all(|c| c.is_ascii_digit())
            {
                return None;
            }
            (h, p.parse::<u16>().ok()?)
        }
        None => (rest, default_port),
    };
    if raw_host.contains(char::is_whitespace) || port == 0 {
        return None;
    }
    let host = if let Ok(ip) = raw_host.parse::<std::net::Ipv4Addr>() {
        ip.to_string()
    } else {
        let trimmed = raw_host.strip_suffix('.').unwrap_or(raw_host);
        if trimmed.is_empty() || !is_valid_hostname(trimmed) {
            return None;
        }
        trimmed.to_ascii_lowercase()
    };
    Some((scheme_norm, host, port))
}

fn is_valid_hostname(host: &str) -> bool {
    if host.len() > 253 {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

/// Hard limits for job input (spec sections 5 and 7).
/// `MAX_DIMENSION_MM` accommodates continuous 80mm/58mm thermal POS rolls
/// (up to 3,276 mm+ Windows spooler forms and long Z-reports).
pub const MAX_JOB_BYTES: usize = 50 * 1024 * 1024; // S4 tested size is also the cap
pub const MAX_PAGES: u32 = 2_000;
pub const MAX_COPIES: u16 = 999;
pub const MAX_DIMENSION_MM: f64 = 5_000.0;

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
    validate_payload_limits(bytes_len, copies)?;
    if pages == 0 || pages > MAX_PAGES {
        return Err(LimitError::TooManyPages);
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
        // HTTP: exact localhost and any IP literal, private or public.
        assert!(origin_is_allowed(
            "http://localhost:8069",
            "http://localhost:8069"
        ));
        assert!(!origin_is_allowed(
            "http://evil.localhost:8069",
            "http://evil.localhost:8069"
        ));
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
        assert_eq!(origin_host_port("https://[evil.com]:8069"), None);
        assert_eq!(origin_host_port("https://::1"), None);
        assert_eq!(origin_host_port("https://."), None);
        assert_eq!(origin_host_port("https://.."), None);
        assert_eq!(origin_host_port("https://a..b"), None);
        assert_eq!(origin_host_port("https://-bad.local"), None);
        assert_eq!(origin_host_port("https://evil%2ecom:8069"), None);
        assert_eq!(origin_host_port("https://\u{0435}xample.com:8069"), None);
        assert_eq!(origin_host_port("https://odoo.local\n:8069"), None);
    }
    #[test]
    fn limits() {
        let a4 = crate::PageSize::a4();
        assert!(validate_job_input(1024, 1, 1, a4).is_ok());
        // Long thermal receipts (e.g. 3276 mm roll) must not be rejected.
        assert!(validate_job_input(1024, 1, 1, crate::PageSize::roll_80(3276.0)).is_ok());
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

//! Security validation that runs in Rust on every IPC command.
//! Rationale (research, 2026-11-02): Tauri app commands are not
//! permission-gated by default, and CVE-2026-42184 showed origin
//! classification itself can be wrong. So we never trust it alone.

/// Exact-origin check. `configured` is the admin-locked Odoo origin, e.g.
/// `https://odoo.company.local:8069`. `actual` is the caller's origin.
/// Rules: exact scheme+host+port match, no userinfo, no path tricks,
/// subdomains never match a parent, and `*.localhost`-style or
/// first-label tricks (CVE-2026-42184) never match.
///
/// Scheme policy (amended 2026-11-03 at the product owner's decision —
/// see ADR-0002 amendment): HTTPS is allowed for any host. Plain HTTP is
/// allowed ONLY when the host is a private, loopback, or link-local IP
/// literal — the common LAN deployment where Odoo is reached by IP
/// (e.g. `http://192.168.1.20:8069`) and no TLS certificate exists.
/// Hostnames and public IPs over HTTP are still rejected.
pub fn origin_is_allowed(configured: &str, actual: &str) -> bool {
    let (Some(c), Some(a)) = (parse_origin(configured), parse_origin(actual)) else {
        return false;
    };
    c == a && scheme_allowed(&c)
}

fn scheme_allowed(origin: &(String, String, u16)) -> bool {
    match origin.0.as_str() {
        "https" => true,
        "http" => host_is_private_ip(&origin.1),
        _ => false,
    }
}

/// True when `host` is an IP literal on a private, loopback, or
/// link-local range (RFC 1918, 127.0.0.0/8, 169.254.0.0/16, ::1,
/// fc00::/7, fe80::/10). Hostnames return false.
fn host_is_private_ip(host: &str) -> bool {
    use std::net::IpAddr;
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    match bare.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        Ok(IpAddr::V6(v6)) => {
            let seg = v6.segments();
            v6 == std::net::Ipv6Addr::LOCALHOST
                || (seg[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (seg[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
        Err(_) => false,
    }
}

/// Minimal strict origin parser: scheme://host[:port] and nothing else.
/// Returns (scheme, host_lowercase, effective_port).
fn parse_origin(s: &str) -> Option<(String, String, u16)> {
    let (scheme, rest) = s.split_once("://")?;
    if rest.is_empty() || rest.contains(['/', '?', '#', '@', '\\']) {
        return None;
    }
    let default_port = if scheme.eq_ignore_ascii_case("https") {
        443
    } else if scheme.eq_ignore_ascii_case("http") {
        80
    } else {
        return None;
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
    Some((scheme.to_ascii_lowercase(), host.to_ascii_lowercase(), port))
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
    const CFG: &str = "https://odoo.company.local:8069";
    #[test]
    fn exact_origin_passes() {
        assert!(origin_is_allowed(CFG, CFG));
    }
    #[test]
    fn evil_subdomain_fails() {
        assert!(!origin_is_allowed(CFG, "https://odoo.evil.com:8069"));
        assert!(!origin_is_allowed(
            "https://app.localhost",
            "https://app.evil.com"
        ));
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
    fn http_private_ip_passes_http_others_fail() {
        // LAN deployments: Odoo by IP over HTTP (owner decision, ADR-0002).
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
        // Still rejected: hostnames and public IPs over HTTP.
        assert!(!origin_is_allowed(
            "http://odoo.company.local:8069",
            "http://odoo.company.local:8069"
        ));
        assert!(!origin_is_allowed(
            "http://8.8.8.8:8069",
            "http://8.8.8.8:8069"
        ));
        assert!(!origin_is_allowed(
            "http://172.32.0.1:8069",
            "http://172.32.0.1:8069"
        ));
        // And an HTTP origin never matches an HTTPS configuration.
        assert!(!origin_is_allowed(
            "https://192.168.1.20:8069",
            "http://192.168.1.20:8069"
        ));
    }
    #[test]
    fn default_port_and_ipv6_equivalence() {
        assert!(origin_is_allowed(
            "https://odoo.example.com",
            "https://odoo.example.com:443"
        ));
        assert!(origin_is_allowed(
            "https://[::1]:8069",
            "https://[::1]:8069"
        ));
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

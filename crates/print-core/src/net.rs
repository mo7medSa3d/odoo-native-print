//! Shared TCP dialer, SSRF link-local/metadata guard, and deadline write helpers.

use std::io::{ErrorKind, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

const MIN_IO_TIMEOUT: Duration = Duration::from_millis(1);

/// Reject link-local and cloud metadata addresses (IPv4 169.254.0.0/16,
/// IPv6 fe80::/10, and IPv4-mapped ::ffff:169.254.0.0/112) to prevent SSRF.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => {
            let segs = v6.segments();
            (segs[0] & 0xffc0) == 0xfe80
                || (segs[0..5] == [0, 0, 0, 0, 0]
                    && segs[5] == 0xffff
                    && segs[6] == u16::from_be_bytes([169, 254]))
        }
    }
}

/// Write `buf` completely before `deadline`, refreshing the socket write
/// timeout on each iteration so stalled peers cannot block past the budget.
pub fn write_all_with_deadline(
    stream: &mut TcpStream,
    mut buf: &[u8],
    deadline: Instant,
) -> std::io::Result<()> {
    while !buf.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "write deadline exceeded",
            ));
        }
        stream.set_write_timeout(Some(remaining.max(MIN_IO_TIMEOUT)))?;
        match stream.write(buf) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::WriteZero,
                    "failed to write whole buffer",
                ));
            }
            Ok(n) => buf = &buf[n..],
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Try connecting to `addrs` in order under an optional shared `deadline`,
/// skipping any address rejected by [`is_blocked_ip`].
pub fn connect_addrs(
    addrs: &[SocketAddr],
    deadline: Option<Instant>,
    fallback_timeout: Duration,
) -> std::io::Result<TcpStream> {
    if addrs.is_empty() {
        return Err(std::io::Error::new(
            ErrorKind::AddrNotAvailable,
            "no address",
        ));
    }
    let mut last_err = None;
    let mut all_blocked = true;
    for addr in addrs {
        if is_blocked_ip(addr.ip()) {
            continue;
        }
        all_blocked = false;
        let remaining = match deadline {
            Some(d) => d.saturating_duration_since(Instant::now()),
            None => fallback_timeout,
        };
        // Bound each dial so one unresponsive address cannot consume the entire
        // shared budget before later addresses are tried.
        let attempt_timeout = remaining.min(fallback_timeout);
        if attempt_timeout.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(addr, attempt_timeout) {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = Some(e),
        }
    }
    if all_blocked {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "refused link-local/metadata address",
        ));
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::new(ErrorKind::TimedOut, "connect timed out")))
}

/// Resolve `host:port` (accepting bracketed IPv6 literals) and connect to the
/// first reachable non-blocked address within `timeout`.
pub fn connect_any(host: &str, port: u16, timeout: Duration) -> std::io::Result<TcpStream> {
    let bare_host = host.trim().trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<_> = (bare_host, port).to_socket_addrs()?.collect();
    let total = timeout.max(MIN_IO_TIMEOUT);
    let per_attempt = (total / (addrs.len().clamp(1, 4) as u32)).max(MIN_IO_TIMEOUT);
    connect_addrs(&addrs, Instant::now().checked_add(total), per_attempt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr, TcpListener};

    #[test]
    fn is_blocked_ip_rejects_link_local_metadata_and_allows_lan() {
        // Cloud metadata & IPv4 link-local
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 0, 1))));
        // IPv6 link-local (fe80::/10)
        assert!(is_blocked_ip(IpAddr::V6("fe80::1".parse::<Ipv6Addr>().unwrap())));
        assert!(is_blocked_ip(IpAddr::V6("febf::1".parse::<Ipv6Addr>().unwrap())));
        // IPv4-mapped metadata address
        assert!(is_blocked_ip(IpAddr::V6(
            "::ffff:169.254.169.254".parse::<Ipv6Addr>().unwrap()
        )));

        // Normal LAN and loopback addresses are allowed
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(!is_blocked_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!is_blocked_ip(IpAddr::V6("fd00::1".parse::<Ipv6Addr>().unwrap())));
        assert!(!is_blocked_ip(IpAddr::V6("fec0::1".parse::<Ipv6Addr>().unwrap())));
        assert!(!is_blocked_ip(IpAddr::V6(
            "::ffff:127.0.0.1".parse::<Ipv6Addr>().unwrap()
        )));
    }

    #[test]
    fn connect_addrs_falls_back_and_enforces_deadline_and_ssrf() {
        let empty_err = connect_addrs(&[], Some(Instant::now()), MIN_IO_TIMEOUT).unwrap_err();
        assert_eq!(empty_err.kind(), ErrorKind::AddrNotAvailable);

        let blocked_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)), 9100);
        let blocked_err = connect_addrs(&[blocked_addr], None, Duration::from_secs(1)).unwrap_err();
        assert_eq!(blocked_err.kind(), ErrorKind::PermissionDenied);

        let live = TcpListener::bind("127.0.0.1:0").unwrap();
        let live_addr = live.local_addr().unwrap();

        let expired_err =
            connect_addrs(&[live_addr], Some(Instant::now()), Duration::from_secs(5)).unwrap_err();
        assert_eq!(expired_err.kind(), ErrorKind::TimedOut);

        // Blocked + closed port followed by live port falls back to live port.
        let dead = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_addr = dead.local_addr().unwrap();
        drop(dead);
        let server = std::thread::spawn(move || {
            let _ = live.accept();
        });
        let stream = connect_addrs(
            &[blocked_addr, dead_addr, live_addr],
            Instant::now().checked_add(Duration::from_secs(10)),
            Duration::from_millis(500),
        )
        .expect("should skip blocked/dead addresses and connect to live address");
        drop(stream);
        let _ = server.join();
    }
}

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Raw TCP 9100 (JetDirect) transport: connect, write all bytes, close.
/// Closing the connection is the end-of-job signal for these printers.
/// Timeouts are always set: a hung printer must never hang the worker.
pub fn send_raw(
    host: &str,
    port: u16,
    bytes: &[u8],
    connect_timeout: Duration,
    io_timeout: Duration,
) -> std::io::Result<()> {
    let addr = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no address"))?;
    let mut stream = TcpStream::connect_timeout(&addr, connect_timeout)?;
    stream.set_write_timeout(Some(io_timeout))?;
    stream.set_read_timeout(Some(io_timeout))?;
    stream.write_all(bytes)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    Ok(())
}

use std::net::ToSocketAddrs;

/// ESC/POS real-time status request (DLE EOT n) — the "sensor read" for
/// thermal printers that support it: n=2 gives offline cause (paper end
/// is bit 5/6 pattern), kept raw here and decoded by decode_dle_eot.
pub fn query_status(host: &str, port: u16, timeout: Duration) -> std::io::Result<u8> {
    let addr = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no address"))?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.write_all(&[0x10, 0x04, 0x02])?; // DLE EOT 2
    let mut buf = [0u8; 1];
    stream.read_exact(&mut buf)?;
    Ok(buf[0])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThermalStatus {
    /// The raw byte, kept because vendors extend these layouts with
    /// quirks; per-printer profiles can interpret extra bits.
    pub raw: u8,
    /// DLE EOT 2 (offline status): bit 3 set means the printer is offline
    /// (cover open, paper end, or an error put it offline).
    pub offline: bool,
    /// Bit 2: cash drawer kick connector pin 3 level (HIGH when set).
    pub drawer_pin_high: bool,
    /// Bit 5: the printer is waiting for online recovery.
    pub waiting_online_recovery: bool,
}

pub fn decode_dle_eot(status: u8) -> ThermalStatus {
    ThermalStatus {
        raw: status,
        offline: status & 0x08 != 0,
        drawer_pin_high: status & 0x04 != 0,
        waiting_online_recovery: status & 0x20 != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn send_to_mock_printer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).unwrap();
            buf
        });
        send_raw(
            "127.0.0.1",
            port,
            b"RAW-PRINT-BYTES",
            Duration::from_secs(2),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(server.join().unwrap(), b"RAW-PRINT-BYTES");
    }
    #[test]
    fn status_round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = [0u8; 3];
            s.read_exact(&mut req).unwrap();
            assert_eq!(req, [0x10, 0x04, 0x02]);
            s.write_all(&[0x12 | 0x08]).unwrap(); // fixed bits + offline
        });
        let raw = query_status("127.0.0.1", port, Duration::from_secs(2)).unwrap();
        server.join().unwrap();
        let st = decode_dle_eot(raw);
        assert!(st.offline);
        assert!(!st.drawer_pin_high);
        assert_eq!(st.raw, 0x1A);
    }
    #[test]
    fn unreachable_printer_errors_fast() {
        let t = std::time::Instant::now();
        let r = send_raw(
            "127.0.0.1",
            1,
            b"x",
            Duration::from_millis(300),
            Duration::from_millis(300),
        );
        assert!(r.is_err());
        assert!(t.elapsed() < Duration::from_secs(5));
    }
}

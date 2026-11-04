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
/// thermal printers that support it. Per Epson's DLE EOT spec the
/// response frames are: n=1 printer status, n=2 offline cause (cover
/// open / error / offline bits), n=3 error cause, n=4 paper-roll sensor
/// status — paper near-end and paper-end live in the n=4 frame, NOT in
/// n=2. Raw bytes are kept and decoded by decode_dle_eot /
/// decode_dle_eot_paper.
pub fn query_status(host: &str, port: u16, timeout: Duration) -> std::io::Result<u8> {
    query_dle_eot(host, port, 2, timeout)
}

/// Query the DLE EOT 4 paper-roll status (near-end / paper-end sensors).
pub fn query_paper_status(host: &str, port: u16, timeout: Duration) -> std::io::Result<u8> {
    query_dle_eot(host, port, 4, timeout)
}

fn query_dle_eot(host: &str, port: u16, n: u8, timeout: Duration) -> std::io::Result<u8> {
    let addr = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no address"))?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.write_all(&[0x10, 0x04, n])?;
    // A status byte has fixed bits (bit 1 and bit 4 set; bits 0 and 7
    // clear). Unsolicited ASB frames or line noise can precede the real
    // response, so skip up to 4 bytes looking for a valid frame instead
    // of decoding whatever arrives first.
    let mut buf = [0u8; 1];
    for _ in 0..4 {
        stream.read_exact(&mut buf)?;
        if dle_eot_frame_valid(buf[0]) {
            return Ok(buf[0]);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "no valid DLE EOT status frame in response",
    ))
}

/// DLE EOT response frames always have bit 1 and bit 4 set and bits 0
/// and 7 clear (Epson DLE EOT specification).
pub fn dle_eot_frame_valid(b: u8) -> bool {
    b & 0x12 == 0x12 && b & 0x81 == 0
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperStatus {
    /// The raw DLE EOT 4 byte (vendors may extend the layout).
    pub raw: u8,
    /// Bits 2-3 both set: paper roll near-end sensor reports the roll
    /// is running out.
    pub paper_near_end: bool,
    /// Bits 5-6 both set: paper end — the roll is out.
    pub paper_end: bool,
}

pub fn decode_dle_eot_paper(status: u8) -> PaperStatus {
    PaperStatus {
        raw: status,
        paper_near_end: (status >> 2) & 0x03 == 0x03,
        paper_end: (status >> 5) & 0x03 == 0x03,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn dle_eot_paper_table() {
        // DLE EOT 4: bits 2-3 = near-end pair, bits 5-6 = paper-end pair.
        let ok = decode_dle_eot_paper(0x12);
        assert!(!ok.paper_near_end && !ok.paper_end);
        let near = decode_dle_eot_paper(0x12 | 0x0C);
        assert!(near.paper_near_end && !near.paper_end);
        let end = decode_dle_eot_paper(0x12 | 0x60);
        assert!(!end.paper_near_end && end.paper_end);
        let both = decode_dle_eot_paper(0x12 | 0x0C | 0x60);
        assert!(both.paper_near_end && both.paper_end);
        // A single bit of a pair is not a sensor reading.
        let half = decode_dle_eot_paper(0x12 | 0x04);
        assert!(!half.paper_near_end);
    }

    #[test]
    fn dle_eot_frame_validation() {
        assert!(dle_eot_frame_valid(0x12)); // minimal valid frame
        assert!(dle_eot_frame_valid(0x16));
        assert!(!dle_eot_frame_valid(0x00)); // fixed bits missing
        assert!(!dle_eot_frame_valid(0x92)); // bit 7 set
        assert!(!dle_eot_frame_valid(0x13)); // bit 0 set
    }

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

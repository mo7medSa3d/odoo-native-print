use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Which phase of a raw TCP send failed. The distinction decides
/// whether a retry is safe: a connect-phase failure proves ZERO
/// bytes reached the printer (nothing could have printed), while
/// a write-phase failure leaves the outcome unknown.
#[derive(Debug)]
pub enum SendPhaseError {
    Connect(std::io::Error),
    Write(std::io::Error),
}

impl std::fmt::Display for SendPhaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendPhaseError::Connect(e) => write!(f, "connect failed: {e}"),
            SendPhaseError::Write(e) => write!(f, "write failed: {e}"),
        }
    }
}

impl std::error::Error for SendPhaseError {}

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
    send_raw_phased(host, port, bytes, connect_timeout, io_timeout).map_err(|e| match e {
        SendPhaseError::Connect(e) | SendPhaseError::Write(e) => e,
    })
}

/// [`send_raw`] with the failure phase preserved (see
/// [`SendPhaseError`]).
pub fn send_raw_phased(
    host: &str,
    port: u16,
    bytes: &[u8],
    connect_timeout: Duration,
    io_timeout: Duration,
) -> Result<(), SendPhaseError> {
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(SendPhaseError::Connect)?
        .next()
        .ok_or_else(|| {
            SendPhaseError::Connect(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "no address",
            ))
        })?;
    let mut stream =
        TcpStream::connect_timeout(&addr, connect_timeout).map_err(SendPhaseError::Connect)?;
    stream
        .set_write_timeout(Some(io_timeout))
        .map_err(SendPhaseError::Write)?;
    stream
        .set_read_timeout(Some(io_timeout))
        .map_err(SendPhaseError::Write)?;
    stream.write_all(bytes).map_err(SendPhaseError::Write)?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(SendPhaseError::Write)?;
    Ok(())
}

use std::net::ToSocketAddrs;

/// ESC/POS real-time status request (DLE EOT n) — the "sensor read" for
/// thermal printers that support it. Per Epson's DLE EOT spec the
/// response frames are: n=1 printer status, n=2 offline cause, n=3
/// error cause (cutter / unrecoverable / auto-recoverable), n=4
/// paper-roll sensor status — paper near-end and paper-end live in
/// the n=4 frame, NOT in n=2. Raw bytes are kept and decoded by
/// decode_dle_eot / decode_dle_eot_paper / decode_dle_eot_error.
///
/// n=2 carries NO cover-open bit: bit 3 is a generic offline flag
/// (cover open, paper end, or an error put the printer offline).
/// Cover state must not be inferred from n=2 alone.
pub fn query_status(host: &str, port: u16, timeout: Duration) -> std::io::Result<u8> {
    query_dle_eot(host, port, 2, timeout)
}

/// Query the DLE EOT 1 printer status (online/offline, drawer, error).
pub fn query_printer_status(host: &str, port: u16, timeout: Duration) -> std::io::Result<u8> {
    query_dle_eot(host, port, 1, timeout)
}

/// Query the DLE EOT 3 error-cause status (cutter, unrecoverable,
/// auto-recoverable errors).
pub fn query_error_status(host: &str, port: u16, timeout: Duration) -> std::io::Result<u8> {
    query_dle_eot(host, port, 3, timeout)
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
    /// Bit 6: an error occurred (paper jam, cutter fault, or other
    /// error put the printer offline). Present in n=1 and n=2.
    pub error_occurred: bool,
}

pub fn decode_dle_eot(status: u8) -> ThermalStatus {
    ThermalStatus {
        raw: status,
        offline: status & 0x08 != 0,
        drawer_pin_high: status & 0x04 != 0,
        waiting_online_recovery: status & 0x20 != 0,
        error_occurred: status & 0x40 != 0,
    }
}

/// DLE EOT 1 (printer status) decoder. Bit 3 offline, bit 2 drawer
/// pin, bit 6 error occurred; bit 5 is set when printing stopped
/// due to paper-end / near-end. The bit layout is the same skeleton
/// as n=2, so the same struct carries it; the query function that
/// produced the byte records which frame it is.
pub fn decode_dle_eot_printer(status: u8) -> ThermalStatus {
    decode_dle_eot(status)
}

/// DLE EOT 3 (error-cause status) decoder (Epson TM reference,
/// Citizen CD-S500 reference guide): bit 2 mechanical/recoverable
/// error, bit 3 auto-cutter error, bit 5 unrecoverable error,
/// bit 6 auto-recoverable error (e.g. head over-temperature).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorStatus {
    pub raw: u8,
    pub mechanical_error: bool,
    pub cutter_error: bool,
    pub unrecoverable_error: bool,
    pub auto_recoverable_error: bool,
}

pub fn decode_dle_eot_error(status: u8) -> ErrorStatus {
    ErrorStatus {
        raw: status,
        mechanical_error: status & 0x04 != 0,
        cutter_error: status & 0x08 != 0,
        unrecoverable_error: status & 0x20 != 0,
        auto_recoverable_error: status & 0x40 != 0,
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

/// The blocking live condition in a set of decoded DLE EOT
/// frames, as (job error code, explanation) — or None when the
/// printer's own sensors say it can print. This is the printer
/// HARDWARE speaking (paper-end sensor, cutter switch, offline
/// state), not a spooler or driver opinion: the distinction the
/// 2026-11-07 review asked for. Near-end and auto-recoverable
/// states (head temperature) deliberately do NOT block: the roll
/// still has paper and the condition clears itself.
pub fn blocking_condition(
    paper: PaperStatus,
    error: ErrorStatus,
    printer: ThermalStatus,
) -> Option<(&'static str, &'static str)> {
    if paper.paper_end {
        return Some((
            "PRINTER_PAPER_OUT",
            "the printer's paper-end sensor reports the roll is out (DLE EOT)",
        ));
    }
    if error.cutter_error {
        return Some((
            "PRINTER_CUTTER_ERROR",
            "the printer reports an auto-cutter error (DLE EOT)",
        ));
    }
    if error.unrecoverable_error || error.mechanical_error {
        return Some((
            "PRINTER_ERROR",
            "the printer reports a mechanical or unrecoverable error (DLE EOT)",
        ));
    }
    if printer.offline || printer.error_occurred {
        return Some((
            "PRINTER_OFFLINE",
            "the printer reports offline — cover open or an error state (DLE EOT)",
        ));
    }
    None
}

/// Query a TCP thermal printer's live sensor state (DLE EOT
/// frames 4, 3, 1) and return its blocking condition, if any.
/// An Err means the printer did not answer the status protocol
/// at all (many clones never do): callers must treat that as
/// "unknown" and print anyway — a status probe may veto a job
/// only on positive sensor evidence, never on silence.
pub fn query_blocking_condition(
    host: &str,
    port: u16,
    timeout: Duration,
) -> std::io::Result<Option<(&'static str, &'static str)>> {
    let paper = decode_dle_eot_paper(query_paper_status(host, port, timeout)?);
    let error = decode_dle_eot_error(query_error_status(host, port, timeout)?);
    let printer = decode_dle_eot_printer(query_printer_status(host, port, timeout)?);
    Ok(blocking_condition(paper, error, printer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn connect_failure_is_connect_phase() {
        // A port nothing listens on: the failure must surface as
        // the Connect phase (zero bytes sent -> safe to retry),
        // never as an ambiguous write failure.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = send_raw_phased(
            "127.0.0.1",
            port,
            b"job",
            Duration::from_millis(500),
            Duration::from_millis(500),
        )
        .unwrap_err();
        assert!(matches!(err, SendPhaseError::Connect(_)), "{err:?}");
    }

    /// A loopback stand-in for a thermal printer: answers each
    /// DLE EOT query with the byte scripted for that frame.
    fn mock_printer(script: [u8; 5]) -> u16 {
        use std::io::{Read, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let mut req = [0u8; 3];
                if stream.read_exact(&mut req).is_err() {
                    continue;
                }
                let n = req[2] as usize;
                if (1..=4).contains(&n) {
                    let _ = stream.write_all(&[script[n]]);
                }
            }
        });
        port
    }

    #[test]
    fn live_probe_reports_paper_out_from_the_sensor() {
        // Frame bytes carry the fixed DLE EOT frame bits (1 and
        // 4) plus the condition bits: 0x72 = paper-end (bits 5-6)
        // on the n=4 paper frame; other frames idle.
        let port = mock_printer([0, 0x12, 0x12, 0x12, 0x72]);
        let found = query_blocking_condition("127.0.0.1", port, Duration::from_secs(2))
            .expect("the mock printer answers DLE EOT");
        assert_eq!(found.map(|(code, _)| code), Some("PRINTER_PAPER_OUT"));
    }

    #[test]
    fn live_probe_healthy_printer_blocks_nothing() {
        let port = mock_printer([0, 0x12, 0x12, 0x12, 0x12]);
        let found = query_blocking_condition("127.0.0.1", port, Duration::from_secs(2))
            .expect("the mock printer answers DLE EOT");
        assert_eq!(found, None);
    }

    #[test]
    fn blocking_condition_ignores_near_end_and_heat() {
        // Near-end (bits 2-3 on n=4) warns but must not block;
        // an auto-recoverable error (bit 6 on n=3) clears itself.
        let near = decode_dle_eot_paper(0x12 | 0x0C);
        assert!(near.paper_near_end && !near.paper_end);
        let hot = decode_dle_eot_error(0x12 | 0x40);
        let idle = decode_dle_eot_printer(0x12);
        assert_eq!(blocking_condition(near, hot, idle), None);
        // Cover-open/offline on n=1 blocks.
        let offline = decode_dle_eot_printer(0x12 | 0x08);
        let ok_paper = decode_dle_eot_paper(0x12);
        let ok_error = decode_dle_eot_error(0x12);
        assert_eq!(
            blocking_condition(ok_paper, ok_error, offline).map(|(c, _)| c),
            Some("PRINTER_OFFLINE")
        );
    }

    #[test]
    fn dle_eot_error_table_n1_n3() {
        // DLE EOT 3 bit table (Epson TM / Citizen references).
        let ok = decode_dle_eot_error(0x12);
        assert!(!ok.mechanical_error && !ok.cutter_error && !ok.unrecoverable_error);
        let cut = decode_dle_eot_error(0x12 | 0x08);
        assert!(cut.cutter_error && !cut.unrecoverable_error);
        let fatal = decode_dle_eot_error(0x12 | 0x20);
        assert!(fatal.unrecoverable_error);
        let hot = decode_dle_eot_error(0x12 | 0x40);
        assert!(hot.auto_recoverable_error);
        let mech = decode_dle_eot_error(0x12 | 0x04);
        assert!(mech.mechanical_error);
        // n=2 gained the bit-6 error flag (previously dropped).
        let st = decode_dle_eot(0x12 | 0x40);
        assert!(st.error_occurred);
        assert!(!st.offline);
        let st1 = decode_dle_eot_printer(0x12 | 0x08 | 0x40);
        assert!(st1.offline && st1.error_occurred);
    }

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

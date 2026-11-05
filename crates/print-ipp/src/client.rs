use crate::proto::{self, op, tag, Attr, IppMessage, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IppError {
    #[error("network error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: status {0}")]
    Http(u16),
    #[error("printer returned IPP status 0x{0:04x}")]
    IppStatus(u16),
    #[error("malformed IPP message: {0}")]
    Parse(String),
    #[error("printer does not accept any format we can produce (it accepts: {0})")]
    NoUsableFormat(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrinterCaps {
    pub name: String,
    /// printer-state enum: 3 idle, 4 processing, 5 stopped.
    pub state: i32,
    pub state_reasons: Vec<String>,
    pub formats_supported: Vec<String>,
    pub media_supported: Vec<String>,
    pub color_supported: bool,
    pub sides_supported: Vec<String>,
    /// printer-is-accepting-jobs; None when the printer did not
    /// report it (unknown, NOT assumed true).
    pub accepting_jobs: Option<bool>,
}

impl PrinterCaps {
    /// Document formats this client can actually PRODUCE. Only PDF:
    /// the job payload is PDF and no PWG Raster / URF / JPEG encoder
    /// exists in the product. Printers that do not accept PDF are
    /// rejected loudly (NoUsableFormat) — sending PDF bytes labeled
    /// as a raster format would print garbage or be rejected by the
    /// device after claiming success here. (Audit D-1.)
    pub const FORMAT_PREFERENCE: [&'static str; 1] = ["application/pdf"];
    pub fn accepts_pdf(&self) -> bool {
        self.formats_supported
            .iter()
            .any(|f| base_format(f) == "application/pdf")
    }
    pub fn choose_format(&self) -> Result<&str, IppError> {
        for pref in Self::FORMAT_PREFERENCE {
            if self
                .formats_supported
                .iter()
                .any(|f| base_format(f) == pref)
            {
                return Ok(pref);
            }
        }
        Err(IppError::NoUsableFormat(self.formats_supported.join(", ")))
    }
    pub fn is_stopped(&self) -> bool {
        self.state == 5
    }
    /// State reasons (RFC 8011 printer-state-reasons, severity suffix
    /// stripped) that mean the device cannot physically print now.
    pub fn blocking_reasons(&self) -> Vec<&str> {
        const BLOCKING: [&str; 8] = [
            "media-empty",
            "media-jam",
            "door-open",
            "cover-open",
            "marker-supply-empty",
            "offline",
            "input-tray-missing",
            "output-full",
        ];
        self.state_reasons
            .iter()
            .map(|r| {
                r.trim()
                    .trim_end_matches("-error")
                    .trim_end_matches("-warning")
                    .trim_end_matches("-report")
            })
            .filter(|r| BLOCKING.contains(r))
            .collect()
    }
    /// True when submission would go to a device that cannot print:
    /// stopped state, not accepting jobs, or a blocking state reason.
    /// An idle printer (state 3) with reasons like "media-low" or
    /// "toner-low" is NOT blocked — those are warnings.
    pub fn is_blocked(&self) -> bool {
        self.is_stopped()
            || self.accepting_jobs == Some(false)
            || !self.blocking_reasons().is_empty()
    }
}

/// Finishing options for an IPP Print-Job request, as IPP keyword
/// values (RFC 8011): `sides` (one-sided / two-sided-long-edge /
/// two-sided-short-edge), `media` (e.g. iso_a4_210x297mm) and
/// `print-color-mode` (color / monochrome).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IppJobOptions {
    pub sides: Option<String>,
    pub media: Option<String>,
    pub color_mode: Option<String>,
    pub media_source: Option<String>,
}

/// Map a Windows tray/bin display name (DC_BINNAMES, e.g. "Tray 1",
/// "Cassette", "Manual Feed") to the IPP `media-source` keyword
/// (PWG 5100 series). Standard names only: anything unrecognised
/// returns None so the caller can fail loudly instead of guessing
/// a tray the printer may not have (owner brief, 2026-11-05).
pub fn tray_to_media_source(tray_name: &str) -> Option<String> {
    let name = tray_name.trim().to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    if name.contains("auto") {
        return Some("auto".to_string());
    }
    if name.contains("manual") {
        return Some("manual".to_string());
    }
    if name.contains("envelope") {
        return Some("envelope-tray".to_string());
    }
    if name.contains("large") {
        return Some("large-capacity".to_string());
    }
    if name.contains("cassette") {
        return Some("cassette".to_string());
    }
    if name.contains("tray") {
        let digits: String = name
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<u32>() {
            if (1..=20).contains(&n) {
                return Some(format!("tray-{n}"));
            }
        }
        return None;
    }
    None
}

/// Build the job-template attributes for a Print-Job request.
fn job_attributes(job_name: &str, copies: u16, options: &IppJobOptions) -> Vec<Attr> {
    let mut job_attrs = vec![Attr::name("job-name", job_name)];
    if copies > 1 {
        job_attrs.push(Attr::integer("copies", copies as i32));
    }
    if let Some(source) = &options.media_source {
        job_attrs.push(Attr::keyword("media-source", source));
    }
    if let Some(sides) = &options.sides {
        job_attrs.push(Attr::keyword("sides", sides));
    }
    if let Some(media) = &options.media {
        job_attrs.push(Attr::keyword("media", media));
    }
    if let Some(mode) = &options.color_mode {
        job_attrs.push(Attr::keyword("print-color-mode", mode));
    }
    job_attrs
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobInfo {
    pub job_id: Option<i32>,
    pub job_state: Option<i32>,
    pub job_uri: Option<String>,
}

pub struct IppClient {
    pub host: String,
    pub port: u16,
    /// HTTP path, usually "/ipp/print".
    pub path: String,
    /// The printer-uri attribute sent inside IPP messages.
    pub printer_uri: String,
    pub timeout: Duration,
    next_request_id: u32,
}

impl IppClient {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
            path: "/ipp/print".to_string(),
            printer_uri: format!("ipp://{host}:{port}/ipp/print"),
            timeout: Duration::from_secs(10),
            next_request_id: 1,
        }
    }

    fn post(&mut self, body: &[u8]) -> Result<Vec<u8>, IppError> {
        let addr = (self.host.as_str(), self.port);
        let sock: std::net::SocketAddr = std::net::ToSocketAddrs::to_socket_addrs(&addr)?
            .next()
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no address")
            })?;
        let mut stream = TcpStream::connect_timeout(&sock, self.timeout)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        let host_hdr = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        };
        let request = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.path,
            host_hdr,
            body.len()
        );
        stream.write_all(request.as_bytes())?;
        stream.write_all(body)?;
        // IPP attribute responses are small; cap the read so a
        // broken or hostile endpoint cannot stream forever and grow
        // memory without bound on the worker thread.
        const MAX_IPP_RESPONSE_BYTES: u64 = 1 << 20;
        let mut raw = Vec::new();
        std::io::Read::by_ref(&mut stream)
            .take(MAX_IPP_RESPONSE_BYTES + 1)
            .read_to_end(&mut raw)?;
        if raw.len() as u64 > MAX_IPP_RESPONSE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "IPP response exceeds the 1 MiB safety cap",
            )
            .into());
        }
        parse_http_response(&raw)
    }

    fn exchange(&mut self, body: &[u8]) -> Result<IppMessage, IppError> {
        let raw = self.post(body)?;
        let msg = proto::parse_message(&raw).map_err(IppError::Parse)?;
        // RFC 8011 section 4.1.6.1: 0x0000..=0x00FF are all `successful-*`
        // status codes (`successful-ok`, `successful-ok-ignored-or-substituted-attributes`,
        // `successful-ok-conflicting-attributes`).
        if msg.code > 0x00FF {
            return Err(IppError::IppStatus(msg.code));
        }
        Ok(msg)
    }

    pub fn get_printer_attributes(&mut self) -> Result<PrinterCaps, IppError> {
        let id = self.next_id();
        let requested = Attr::multi(
            "requested-attributes",
            vec![
                Value::Text(tag::KEYWORD, "printer-name".into()),
                Value::Text(tag::KEYWORD, "printer-state".into()),
                Value::Text(tag::KEYWORD, "printer-state-reasons".into()),
                Value::Text(tag::KEYWORD, "printer-is-accepting-jobs".into()),
                Value::Text(tag::KEYWORD, "document-format-supported".into()),
                Value::Text(tag::KEYWORD, "media-supported".into()),
                Value::Text(tag::KEYWORD, "color-supported".into()),
                Value::Text(tag::KEYWORD, "sides-supported".into()),
            ],
        );
        let body = proto::encode_request(
            op::GET_PRINTER_ATTRIBUTES,
            id,
            &self.printer_uri.clone(),
            &[requested],
            &[],
            None,
        );
        let msg = self.exchange(&body)?;
        Ok(PrinterCaps {
            name: msg
                .texts(tag::GROUP_PRINTER, "printer-name")
                .into_iter()
                .next()
                .unwrap_or_default(),
            state: msg.int(tag::GROUP_PRINTER, "printer-state").unwrap_or(-1),
            state_reasons: msg.texts(tag::GROUP_PRINTER, "printer-state-reasons"),
            formats_supported: msg.texts(tag::GROUP_PRINTER, "document-format-supported"),
            media_supported: msg.texts(tag::GROUP_PRINTER, "media-supported"),
            color_supported: msg
                .bool(tag::GROUP_PRINTER, "color-supported")
                .unwrap_or(false),
            sides_supported: msg.texts(tag::GROUP_PRINTER, "sides-supported"),
            accepting_jobs: msg.bool(tag::GROUP_PRINTER, "printer-is-accepting-jobs"),
        })
    }

    /// Submit a document. `format` must be a value the printer listed in
    /// document-format-supported (see PrinterCaps::choose_format); it is
    /// emitted exactly once (see the CUPS #1457 pitfall in the research).
    pub fn print_job(
        &mut self,
        document: &[u8],
        format: &str,
        job_name: &str,
        copies: u16,
    ) -> Result<JobInfo, IppError> {
        self.print_job_with_options(
            document,
            format,
            job_name,
            copies,
            &IppJobOptions::default(),
        )
    }

    /// Submit a document with finishing options (sides / media /
    /// colour mode as IPP keyword values). The caller validates the
    /// values against the printer's reported capabilities first;
    /// `None` fields are simply not sent.
    pub fn print_job_with_options(
        &mut self,
        document: &[u8],
        format: &str,
        job_name: &str,
        copies: u16,
        options: &IppJobOptions,
    ) -> Result<JobInfo, IppError> {
        let id = self.next_id();
        let extra = [
            Attr::name("requesting-user-name", "odoo-native-print"),
            Attr::new(
                "document-format",
                Value::Text(tag::MIME, format.to_string()),
            ),
        ];
        let job_attrs = job_attributes(job_name, copies, options);
        let body = proto::encode_request(
            op::PRINT_JOB,
            id,
            &self.printer_uri.clone(),
            &extra,
            &job_attrs,
            Some(document),
        );
        let msg = self.exchange(&body)?;
        Ok(JobInfo {
            job_id: msg.int(tag::GROUP_JOB, "job-id"),
            job_state: msg.int(tag::GROUP_JOB, "job-state"),
            job_uri: msg.texts(tag::GROUP_JOB, "job-uri").into_iter().next(),
        })
    }

    fn next_id(&mut self) -> u32 {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        id
    }
}

/// MIME base type of a document-format value: parameters after ';'
/// and case are presentation details, not a different format.
fn base_format(f: &str) -> String {
    f.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn parse_http_response(mut raw: &[u8]) -> Result<Vec<u8>, IppError> {
    loop {
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| IppError::Parse("no HTTP header terminator".into()))?;
        let headers = String::from_utf8_lossy(&raw[..split]).to_string();
        let status: u16 = headers
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| IppError::Parse("no HTTP status".into()))?;
        // Skip interim 1xx responses (e.g. `HTTP/1.1 100 Continue`) that CUPS
        // and embedded IPP servers frequently prepend before `HTTP/1.1 200 OK`.
        if (100..200).contains(&status) {
            raw = &raw[split + 4..];
            continue;
        }
        if status != 200 {
            return Err(IppError::Http(status));
        }
        let body = &raw[split + 4..];
        let lower = headers.to_lowercase();
        return if lower.contains("transfer-encoding: chunked") {
            dechunk(body).map_err(IppError::Parse)
        } else if let Some(len) = lower
            .lines()
            .find(|l| l.starts_with("content-length:"))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            if body.len() < len {
                return Err(IppError::Parse("short HTTP body".into()));
            }
            Ok(body[..len].to_vec())
        } else {
            Ok(body.to_vec())
        };
    }
}

fn dechunk(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let line_end = body[pos..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("bad chunk header")?
            + pos;
        let size_str = String::from_utf8_lossy(&body[pos..line_end]).to_string();
        let size = usize::from_str_radix(size_str.trim().split(';').next().unwrap_or(""), 16)
            .map_err(|_| "bad chunk size")?;
        if size == 0 {
            break;
        }
        let start = line_end + 2;
        let end = start + size;
        if end + 2 > body.len() {
            return Err("truncated chunk".into());
        }
        out.extend_from_slice(&body[start..end]);
        pos = end + 2;
    }
    Ok(out)
}

#[cfg(test)]
mod job_attribute_tests {
    use super::*;

    #[test]
    fn tray_names_map_to_media_source_keywords() {
        // Windows DC_BINNAMES values -> IPP media-source keywords.
        assert_eq!(tray_to_media_source("Tray 1").as_deref(), Some("tray-1"));
        assert_eq!(tray_to_media_source("Tray 2").as_deref(), Some("tray-2"));
        assert_eq!(
            tray_to_media_source("Automatically Select").as_deref(),
            Some("auto")
        );
        assert_eq!(
            tray_to_media_source("Manual Feed").as_deref(),
            Some("manual")
        );
        assert_eq!(
            tray_to_media_source("Cassette 1").as_deref(),
            Some("cassette")
        );
        assert_eq!(
            tray_to_media_source("Envelope Tray").as_deref(),
            Some("envelope-tray")
        );
        // Unmappable names stay unmappable (caller must fail loudly).
        assert_eq!(tray_to_media_source("Bypass"), None);
        assert_eq!(tray_to_media_source("Tray 99"), None);
        assert_eq!(tray_to_media_source(""), None);
        // And the mapped value lands in the job attributes.
        let attrs = job_attributes(
            "job",
            1,
            &IppJobOptions {
                media_source: tray_to_media_source("Tray 1"),
                ..Default::default()
            },
        );
        assert!(attrs.iter().any(|a| a.name == "media-source"
            && matches!(&a.values[..], [crate::proto::Value::Text(_, s)] if s == "tray-1")));
    }

    #[test]
    fn job_attributes_carry_finishing_options() {
        let opts = IppJobOptions {
            sides: Some("two-sided-long-edge".to_string()),
            media: Some("iso_a4_210x297mm".to_string()),
            color_mode: Some("monochrome".to_string()),
            media_source: None,
        };
        let attrs = job_attributes("Report", 2, &opts);
        let names: Vec<&str> = attrs.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            ["job-name", "copies", "sides", "media", "print-color-mode"]
        );
        let empty = job_attributes("Report", 1, &IppJobOptions::default());
        assert_eq!(empty.len(), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{encode_response, parse_message};
    use std::net::TcpListener;

    fn mock_printer(response: Vec<u8>) -> (u16, std::thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 4096];
            let header_end = loop {
                let n = s.read(&mut buf).unwrap();
                raw.extend_from_slice(&buf[..n]);
                if let Some(p) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break p + 4;
                }
            };
            let headers = String::from_utf8_lossy(&raw[..header_end]).to_string();
            let len: usize = headers
                .to_lowercase()
                .lines()
                .find(|l| l.starts_with("content-length:"))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            while raw.len() < header_end + len {
                let n = s.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
            }
            let body = raw[header_end..header_end + len].to_vec();
            let http = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            s.write_all(http.as_bytes()).unwrap();
            s.write_all(&response).unwrap();
            body
        });
        (port, handle)
    }

    fn caps_response() -> Vec<u8> {
        encode_response(
            0,
            1,
            &[(
                tag::GROUP_PRINTER,
                vec![
                    Attr::name("printer-name", "MockJet"),
                    Attr::enumeration("printer-state", 3),
                    Attr::multi(
                        "document-format-supported",
                        vec![
                            Value::Text(tag::MIME, "application/pdf".into()),
                            Value::Text(tag::MIME, "image/pwg-raster".into()),
                        ],
                    ),
                    Attr::multi(
                        "media-supported",
                        vec![
                            Value::Text(tag::KEYWORD, "iso_a4_210x297mm".into()),
                            Value::Text(tag::KEYWORD, "iso_a5_148x210mm".into()),
                        ],
                    ),
                    Attr::boolean("color-supported", true),
                ],
            )],
        )
    }

    #[test]
    fn get_attributes_against_mock() {
        let (port, server) = mock_printer(caps_response());
        let mut client = IppClient::new("127.0.0.1", port);
        let caps = client.get_printer_attributes().unwrap();
        let request = server.join().unwrap();
        let parsed = parse_message(&request).unwrap();
        assert_eq!(parsed.code, op::GET_PRINTER_ATTRIBUTES);
        assert_eq!(caps.name, "MockJet");
        assert_eq!(caps.state, 3);
        assert!(caps.accepts_pdf());
        assert_eq!(caps.choose_format().unwrap(), "application/pdf");
        assert_eq!(caps.media_supported.len(), 2);
        assert!(caps.color_supported);
    }

    #[test]
    fn print_job_accepts_0x0001_substituted_attributes_and_100_continue() {
        // Status 0x0001 = successful-ok-ignored-or-substituted-attributes
        let job_response = encode_response(
            0x0001,
            2,
            &[(
                tag::GROUP_JOB,
                vec![
                    Attr::integer("job-id", 42),
                    Attr::enumeration("job-state", 5),
                ],
            )],
        );
        let raw_http = [
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\n\r\n"
                .as_slice(),
            &job_response,
        ]
        .concat();
        let body = parse_http_response(&raw_http).unwrap();
        let parsed = parse_message(&body).unwrap();
        assert!(parsed.code <= 0x00FF);
    }

    #[test]
    fn print_job_against_mock() {
        let job_response = encode_response(
            0,
            2,
            &[(
                tag::GROUP_JOB,
                vec![
                    Attr::integer("job-id", 42),
                    Attr::enumeration("job-state", 5),
                ],
            )],
        );
        let (port, server) = mock_printer(job_response);
        let mut client = IppClient::new("127.0.0.1", port);
        let pdf = b"%PDF-1.4 fake but byte-exact document";
        let info = client
            .print_job(pdf, "application/pdf", "Invoice 7", 2)
            .unwrap();
        let request = server.join().unwrap();
        let parsed = parse_message(&request).unwrap();
        assert_eq!(parsed.code, op::PRINT_JOB);
        assert_eq!(parsed.data, pdf);
        assert_eq!(parsed.count_attr("document-format"), 1);
        assert_eq!(info.job_id, Some(42));
        assert_eq!(info.job_state, Some(5));
    }

    #[test]
    fn format_choice_without_pdf_is_rejected_not_mislabeled() {
        // Audit D-1: this client produces PDF only. A raster-only
        // printer must be rejected loudly, never sent PDF bytes
        // labeled as image/urf.
        let caps = PrinterCaps {
            name: "RasterOnly".into(),
            state: 3,
            state_reasons: vec![],
            formats_supported: vec!["image/urf".into(), "image/jpeg".into()],
            media_supported: vec![],
            color_supported: false,
            sides_supported: vec![],
            accepting_jobs: None,
        };
        assert!(!caps.accepts_pdf());
        assert!(caps.choose_format().is_err());
        let none = PrinterCaps {
            formats_supported: vec!["text/plain".into()],
            ..caps.clone()
        };
        assert!(none.choose_format().is_err());
        // Format values with parameters / different case still match.
        let fancy = PrinterCaps {
            formats_supported: vec!["Application/PDF; version=1.7".into()],
            ..caps
        };
        assert!(fancy.accepts_pdf());
        assert_eq!(fancy.choose_format().unwrap(), "application/pdf");
    }

    #[test]
    fn blocked_state_reasons() {
        let base = PrinterCaps {
            name: "Office".into(),
            state: 3,
            state_reasons: vec![],
            formats_supported: vec!["application/pdf".into()],
            media_supported: vec![],
            color_supported: false,
            sides_supported: vec![],
            accepting_jobs: Some(true),
        };
        assert!(!base.is_blocked());
        let low = PrinterCaps {
            state_reasons: vec!["media-low-warning".into(), "toner-low".into()],
            ..base.clone()
        };
        assert!(!low.is_blocked(), "warnings do not block");
        let empty = PrinterCaps {
            state_reasons: vec!["media-empty-error".into()],
            ..base.clone()
        };
        assert!(empty.is_blocked());
        assert_eq!(empty.blocking_reasons(), vec!["media-empty"]);
        let jam = PrinterCaps {
            state_reasons: vec!["media-jam".into()],
            ..base.clone()
        };
        assert!(jam.is_blocked());
        let not_accepting = PrinterCaps {
            accepting_jobs: Some(false),
            ..base.clone()
        };
        assert!(not_accepting.is_blocked());
        let stopped = PrinterCaps { state: 5, ..base };
        assert!(stopped.is_blocked());
    }

    #[test]
    fn parser_survives_garbage() {
        assert!(parse_message(b"").is_err());
        assert!(parse_message(b"\x01\x01\x00").is_err());
        assert!(parse_message(&[0xFF; 64]).is_err());
    }
}

use crate::proto::{self, op, tag, Attr, IppMessage, Value};
use print_core::net::{connect_any, write_all_with_deadline};
use std::io::{Read, Write};
use std::time::{Duration, Instant};
use thiserror::Error;

const MIN_IPP_TIMEOUT: Duration = Duration::from_millis(1);

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
            .any(|f| base_format(f).eq_ignore_ascii_case("application/pdf"))
    }
    pub fn choose_format(&self) -> Result<&str, IppError> {
        for pref in Self::FORMAT_PREFERENCE {
            if self
                .formats_supported
                .iter()
                .any(|f| base_format(f).eq_ignore_ascii_case(pref))
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
            .filter_map(|r| {
                let t = r.trim();
                // RFC 8011: -warning and -report severities are advisory, never hard blockers.
                if t.ends_with("-warning") || t.ends_with("-report") {
                    return None;
                }
                let base = t.trim_end_matches("-error");
                BLOCKING.contains(&base).then_some(base)
            })
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
        let bare_host = host.trim().trim_start_matches('[').trim_end_matches(']');
        let uri_host = if bare_host.contains(':') {
            format!("[{bare_host}]")
        } else {
            bare_host.to_string()
        };
        Self {
            host: bare_host.to_string(),
            port,
            path: "/ipp/print".to_string(),
            printer_uri: format!("ipp://{uri_host}:{port}/ipp/print"),
            timeout: Duration::from_secs(30),
            next_request_id: 1,
        }
    }

    fn post(&mut self, body: &[u8]) -> Result<Vec<u8>, IppError> {
        let bare_host = self
            .host
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']');
        if bare_host.is_empty()
            || bare_host
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
            || self
                .path
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(IppError::Parse("invalid host or path".into()));
        }
        let timeout = self.timeout.max(MIN_IPP_TIMEOUT);
        let mut stream = connect_any(bare_host, self.port, timeout)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let host_hdr = if bare_host.contains(':') {
            format!("[{bare_host}]:{}", self.port)
        } else {
            format!("{bare_host}:{}", self.port)
        };
        let path = if self.path.starts_with('/') {
            self.path.as_str()
        } else {
            "/ipp/print"
        };
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {host_hdr}\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let write_deadline = Instant::now() + timeout.max(Duration::from_secs(30));
        write_all_with_deadline(&mut stream, request.as_bytes(), write_deadline)?;
        write_all_with_deadline(&mut stream, body, write_deadline)?;
        stream.flush()?;
        // IPP attribute responses are small; cap the read so a
        // broken or hostile endpoint cannot stream forever and grow
        // memory without bound on the worker thread.
        const MAX_IPP_RESPONSE_BYTES: usize = 1 << 20;
        let read_deadline = std::time::Instant::now().checked_add(timeout);
        let mut raw = Vec::new();
        let mut buf = [0u8; 8192];
        let mut parsed_header: Option<ParsedHttpHeader> = None;
        loop {
            if raw.len() > MAX_IPP_RESPONSE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "IPP response exceeds the 1 MiB safety cap",
                )
                .into());
            }
            if parsed_header.is_none() {
                parsed_header = split_http_response(&raw)?;
            }
            if let Some((status, framing, body_offset)) = parsed_header {
                if http_body_complete(status, framing, &raw[body_offset..]) {
                    break;
                }
            }
            if let Some(deadline) = read_deadline {
                let remaining_time = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining_time.is_zero() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "IPP response timed out",
                    )
                    .into());
                }
                stream.set_read_timeout(Some(remaining_time.max(MIN_IPP_TIMEOUT)))?;
            }
            let remaining = (MAX_IPP_RESPONSE_BYTES + 1) - raw.len();
            let to_read = buf.len().min(remaining);
            match stream.read(&mut buf[..to_read]) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        let (status, framing, body_offset) = match parsed_header {
            Some(header) => header,
            None => split_http_response(&raw)?
                .ok_or_else(|| IppError::Parse("no HTTP header terminator".into()))?,
        };
        finish_http_response(status, framing, &raw[body_offset..])
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
            &self.printer_uri,
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
            &self.printer_uri,
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
fn base_format(f: &str) -> &str {
    f.split(';').next().unwrap_or("").trim()
}

#[derive(Clone, Copy)]
enum HttpFraming {
    Chunked,
    ContentLength(usize),
    Close,
}

type ParsedHttpHeader = (u16, HttpFraming, usize);

fn split_http_response(raw: &[u8]) -> Result<Option<ParsedHttpHeader>, IppError> {
    let mut offset = 0usize;
    loop {
        let rest = &raw[offset..];
        let Some(split) = rest.windows(4).position(|w| w == b"\r\n\r\n") else {
            return Ok(None);
        };
        let headers = String::from_utf8_lossy(&rest[..split]);
        let status: u16 = headers
            .lines()
            .next()
            .and_then(|line| {
                let mut parts = line.split_whitespace();
                let proto = parts.next()?;
                if !proto.starts_with("HTTP/") {
                    return None;
                }
                parts.next()?.parse().ok()
            })
            .ok_or_else(|| IppError::Parse("no HTTP status".into()))?;
        // Skip interim 1xx responses (e.g. `HTTP/1.1 100 Continue`) that CUPS
        // and embedded IPP servers frequently prepend before `HTTP/1.1 200 OK`.
        if (100..200).contains(&status) {
            offset += split + 4;
            continue;
        }
        let body_offset = offset + split + 4;
        let lower = headers.to_ascii_lowercase();
        let mut has_transfer_encoding = false;
        let mut is_chunked = false;
        let mut content_length: Option<usize> = None;
        for line in lower.lines().skip(1) {
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            match k.trim() {
                "transfer-encoding" => {
                    has_transfer_encoding = true;
                    let mut saw_token = false;
                    for t in v.split(',').map(str::trim).filter(|t| !t.is_empty()) {
                        saw_token = true;
                        if t == "chunked" {
                            is_chunked = true;
                        } else {
                            return Err(IppError::Parse("unsupported Transfer-Encoding".into()));
                        }
                    }
                    if !saw_token {
                        return Err(IppError::Parse("empty Transfer-Encoding".into()));
                    }
                }
                "content-length" => {
                    for part in v.split(',') {
                        let s = part.trim();
                        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                            return Err(IppError::Parse("invalid Content-Length".into()));
                        }
                        let len = s
                            .parse::<usize>()
                            .map_err(|_| IppError::Parse("invalid Content-Length".into()))?;
                        match content_length {
                            Some(prev) if prev != len => {
                                return Err(IppError::Parse(
                                    "conflicting Content-Length headers".into(),
                                ));
                            }
                            _ => content_length = Some(len),
                        }
                    }
                }
                _ => {}
            }
        }
        // RFC 9112 §6.3: reject messages carrying both Transfer-Encoding and Content-Length.
        if has_transfer_encoding && content_length.is_some() {
            return Err(IppError::Parse(
                "conflicting Transfer-Encoding and Content-Length".into(),
            ));
        }
        let framing = if is_chunked {
            HttpFraming::Chunked
        } else if let Some(len) = content_length {
            HttpFraming::ContentLength(len)
        } else {
            HttpFraming::Close
        };
        return Ok(Some((status, framing, body_offset)));
    }
}

fn chunked_body_complete(body: &[u8]) -> bool {
    // Fast-path: every RFC 9112 terminated chunked body (with or without
    // trailers) is at least 5 bytes ("0\r\n\r\n") and ends with "\r\n\r\n".
    // Skip full chunk walking on partial read() buffers to avoid O(N^2) rescans.
    body.len() >= 5 && body.ends_with(b"\r\n\r\n") && dechunk_into(body, None).is_ok()
}

fn http_body_complete(status: u16, framing: HttpFraming, body: &[u8]) -> bool {
    if status != 200 {
        return true;
    }
    match framing {
        HttpFraming::Chunked => chunked_body_complete(body),
        HttpFraming::ContentLength(len) => body.len() >= len,
        HttpFraming::Close => false,
    }
}

#[cfg(test)]
fn http_response_complete(raw: &[u8]) -> bool {
    match split_http_response(raw) {
        Ok(None) => false,
        Err(_) => true,
        Ok(Some((status, framing, body_offset))) => {
            http_body_complete(status, framing, &raw[body_offset..])
        }
    }
}

fn finish_http_response(
    status: u16,
    framing: HttpFraming,
    body: &[u8],
) -> Result<Vec<u8>, IppError> {
    if status != 200 {
        return Err(IppError::Http(status));
    }
    match framing {
        HttpFraming::Chunked => dechunk(body).map_err(IppError::Parse),
        HttpFraming::ContentLength(len) => {
            if body.len() < len {
                return Err(IppError::Parse("short HTTP body".into()));
            }
            Ok(body[..len].to_vec())
        }
        HttpFraming::Close => Ok(body.to_vec()),
    }
}

pub fn parse_http_response(raw: &[u8]) -> Result<Vec<u8>, IppError> {
    let (status, framing, body_offset) = split_http_response(raw)?
        .ok_or_else(|| IppError::Parse("no HTTP header terminator".into()))?;
    finish_http_response(status, framing, &raw[body_offset..])
}

fn dechunk_into(body: &[u8], mut out: Option<&mut Vec<u8>>) -> Result<(), String> {
    let mut pos = 0usize;
    loop {
        let line_end = body[pos..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("bad chunk header")?
            + pos;
        let size_part = body[pos..line_end]
            .split(|&b| b == b';')
            .next()
            .unwrap_or(b"");
        let size_hex = std::str::from_utf8(size_part)
            .map_err(|_| "bad chunk size")?
            .trim();
        if size_hex.is_empty() || !size_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("bad chunk size".into());
        }
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| "bad chunk size")?;
        if size == 0 {
            let tail = &body[line_end + 2..];
            if tail.starts_with(b"\r\n") || tail.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            return Err("unterminated chunked body".into());
        }
        let start = line_end + 2;
        let end = start.checked_add(size).ok_or("chunk size overflow")?;
        if end > body.len() {
            return Err("truncated chunk".into());
        }
        if body.get(end..end + 2) != Some(b"\r\n") {
            return Err("bad chunk terminator".into());
        }
        if let Some(ref mut buf) = out {
            buf.extend_from_slice(&body[start..end]);
        }
        pos = end + 2;
    }
    Ok(())
}

fn dechunk(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(body.len());
    dechunk_into(body, Some(&mut out))?;
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
    use print_core::net::{connect_addrs, is_blocked_ip};
    use std::net::{SocketAddr, TcpListener};

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
    fn print_job_accepts_0x0001_0x0002_warnings_and_rejects_0x0400() {
        for status_code in [0x0001u16, 0x0002u16] {
            let job_response = encode_response(
                status_code,
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
            let info = client
                .print_job(b"%PDF-1.4 test", "application/pdf", "Invoice", 1)
                .expect("0x0000..=0x00FF status codes must succeed");
            let _ = server.join().unwrap();
            assert_eq!(info.job_id, Some(42));
        }

        let err_response = encode_response(0x0400, 3, &[]);
        let (port, server) = mock_printer(err_response);
        let mut client = IppClient::new("127.0.0.1", port);
        let err = client
            .print_job(b"%PDF-1.4 test", "application/pdf", "Invoice", 1)
            .unwrap_err();
        let _ = server.join().unwrap();
        assert!(matches!(err, IppError::IppStatus(0x0400)));
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
    fn parse_chunked_http_response() {
        let payload = b"helloipp";
        let raw = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            payload.len(),
            std::str::from_utf8(payload).unwrap()
        );
        let body = parse_http_response(raw.as_bytes()).unwrap();
        assert_eq!(body, payload);
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
        assert!(dechunk(b"5\r\nhelloXX0\r\n\r\n").is_err());
        assert!(dechunk(b"5\r\nhello\r\n0\r\n").is_err());
        assert!(dechunk(b"+5\r\nhello\r\n0\r\n\r\n").is_err());
        assert!(parse_http_response(b"HTTP/1.1\r\n200 OK\r\n\r\n").is_err());
    }

    #[test]
    fn http_framing_rejects_smuggling_and_conflicting_content_length() {
        // Conflicting duplicate Content-Length headers (RFC 9112 §6.3).
        assert!(matches!(
            parse_http_response(
                b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Length: 8\r\n\r\n12345678"
            ),
            Err(IppError::Parse(_))
        ));
        // Conflicting comma-separated Content-Length values.
        assert!(matches!(
            parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 4, 8\r\n\r\n12345678"),
            Err(IppError::Parse(_))
        ));
        // Identical duplicate Content-Length values remain accepted.
        assert_eq!(
            parse_http_response(
                b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Length: 4\r\n\r\ntest"
            )
            .unwrap(),
            b"test"
        );
        // Malformed / signed Content-Length must fail, not fall back to Close.
        assert!(matches!(
            parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: +4\r\n\r\ntest"),
            Err(IppError::Parse(_))
        ));
        assert!(matches!(
            parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: abc\r\n\r\ntest"),
            Err(IppError::Parse(_))
        ));
        // Transfer-Encoding + Content-Length smuggling combination.
        assert!(matches!(
            parse_http_response(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 4\r\n\r\n4\r\ntest\r\n0\r\n\r\n"
            ),
            Err(IppError::Parse(_))
        ));
        // Unsupported Transfer-Encoding must fail, not fall back to Close.
        assert!(matches!(
            parse_http_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\ndata"),
            Err(IppError::Parse(_))
        ));
        // Non-HTTP status line must be rejected.
        assert!(matches!(
            parse_http_response(b"FOO 200 OK\r\n\r\ndata"),
            Err(IppError::Parse(_))
        ));
    }

    #[test]
    fn post_rejects_crlf_in_host_or_path() {
        let mut bad_host = IppClient::new("127.0.0.1\r\nEvil: 1", 631);
        assert!(matches!(
            bad_host.get_printer_attributes(),
            Err(IppError::Parse(_))
        ));
        let mut bad_path = IppClient::new("127.0.0.1", 631);
        bad_path.path = "/ipp/print\r\nEvil: 1".into();
        assert!(matches!(
            bad_path.get_printer_attributes(),
            Err(IppError::Parse(_))
        ));
    }

    #[test]
    fn post_returns_promptly_when_server_keeps_socket_open() {
        let response = caps_response();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
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
            let http = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n",
                response.len()
            );
            s.write_all(http.as_bytes()).unwrap();
            s.write_all(&response).unwrap();
            s.flush().unwrap();
            // Intentionally keep the socket open without shutdown/drop.
            std::thread::sleep(Duration::from_secs(2));
        });

        let mut client = IppClient::new("127.0.0.1", port);
        client.timeout = Duration::from_secs(2);
        let start = std::time::Instant::now();
        let caps = client.get_printer_attributes().unwrap();
        let elapsed = start.elapsed();
        assert_eq!(caps.name, "MockJet");
        assert!(
            elapsed < Duration::from_millis(1000),
            "expected prompt return without waiting for EOF/timeout, took {elapsed:?}"
        );
        let _ = server.join();
    }

    #[test]
    fn chunked_fast_path_handles_partial_trailers_and_embedded_terminators() {
        let hdr = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        // Partial chunk (missing terminator): must not be treated as complete.
        assert!(!http_response_complete(
            &[hdr.as_slice(), b"5\r\nhello\r\n"].concat()
        ));
        assert!(!http_response_complete(
            &[hdr.as_slice(), b"5\r\nhello\r\n0\r\n"].concat()
        ));
        // Payload that itself ends with "0\r\n\r\n" inside an incomplete chunk:
        // suffix check passes, full zero-alloc scan rejects it as incomplete.
        assert!(!chunked_body_complete(b"a\r\nabcd0\r\n\r\n"));
        // Complete multi-chunk body with chunk extension and trailer.
        let with_trailer = b"5;ext=1\r\nhello\r\n3\r\nipp\r\n0\r\nX-Check: ok\r\n\r\n";
        assert!(chunked_body_complete(with_trailer));
        let full = [hdr.as_slice(), with_trailer.as_slice()].concat();
        assert!(http_response_complete(&full));
        assert_eq!(parse_http_response(&full).unwrap(), b"helloipp");
    }

    #[test]
    fn connect_addrs_enforces_shared_deadline_and_falls_back() {
        let empty_err = connect_addrs(&[], Some(Instant::now()), MIN_IPP_TIMEOUT).unwrap_err();
        assert_eq!(empty_err.kind(), std::io::ErrorKind::AddrNotAvailable);

        let live = TcpListener::bind("127.0.0.1:0").unwrap();
        let live_addr = live.local_addr().unwrap();

        // Already-expired shared deadline stops immediately without dialing live_addr.
        let expired = Instant::now();
        let expired_err =
            connect_addrs(&[live_addr], Some(expired), Duration::from_secs(5)).unwrap_err();
        assert_eq!(expired_err.kind(), std::io::ErrorKind::TimedOut);

        // Closed port followed by live port falls back within the shared budget.
        let dead = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_addr = dead.local_addr().unwrap();
        drop(dead);
        let server = std::thread::spawn(move || {
            let _ = live.accept();
        });
        let total = Duration::from_secs(10);
        let per_attempt = Duration::from_millis(500);
        let stream = connect_addrs(
            &[dead_addr, live_addr],
            Instant::now().checked_add(total),
            per_attempt,
        )
        .expect("second address should succeed after first fails");
        drop(stream);
        let _ = server.join();
    }

    #[test]
    fn blocked_ips_reject_link_local_and_metadata() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V6(
            "fe80::1".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(is_blocked_ip(IpAddr::V6(
            "febf::1".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(is_blocked_ip(IpAddr::V6(
            "::ffff:169.254.169.254".parse::<Ipv6Addr>().unwrap()
        )));

        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))));
        assert!(!is_blocked_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!is_blocked_ip(IpAddr::V6(
            "fd00::1".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(!is_blocked_ip(IpAddr::V6(
            "fec0::1".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(!is_blocked_ip(IpAddr::V6(
            "::ffff:127.0.0.1".parse::<Ipv6Addr>().unwrap()
        )));

        let blocked_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)), 631);
        let err = connect_addrs(&[blocked_addr], None, Duration::from_secs(1)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn post_handles_split_tcp_chunks_with_cached_headers() {
        let response = caps_response();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 4096];
            while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = s.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
            }
            let http = format!(
                "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            let mut full = http.into_bytes();
            full.extend_from_slice(&response);
            for chunk in full.chunks(17) {
                s.write_all(chunk).unwrap();
                s.flush().unwrap();
            }
        });
        let mut client = IppClient::new("127.0.0.1", port);
        let caps = client.get_printer_attributes().unwrap();
        assert_eq!(caps.name, "MockJet");
        server.join().unwrap();
    }
}

//! IPP message encoding/parsing (RFC 8010). See
//! docs/research/ipp-protocol.md for the verified constants and pitfalls.

pub mod tag {
    pub const GROUP_OPERATION: u8 = 0x01;
    pub const GROUP_JOB: u8 = 0x02;
    pub const END_OF_ATTRIBUTES: u8 = 0x03;
    pub const GROUP_PRINTER: u8 = 0x04;
    pub const INTEGER: u8 = 0x21;
    pub const BOOLEAN: u8 = 0x22;
    pub const ENUM: u8 = 0x23;
    pub const TEXT: u8 = 0x41;
    pub const NAME: u8 = 0x42;
    pub const KEYWORD: u8 = 0x44;
    pub const URI: u8 = 0x45;
    pub const CHARSET: u8 = 0x47;
    pub const LANGUAGE: u8 = 0x48;
    pub const MIME: u8 = 0x49;
}

pub mod op {
    pub const PRINT_JOB: u16 = 0x0002;
    pub const CANCEL_JOB: u16 = 0x0008;
    pub const GET_JOB_ATTRIBUTES: u16 = 0x0009;
    pub const GET_PRINTER_ATTRIBUTES: u16 = 0x000B;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Integer(i32),
    Boolean(bool),
    Enum(i32),
    Text(u8, String), // value tag (TEXT/NAME/KEYWORD/URI/CHARSET/LANGUAGE/MIME)
}

impl Value {
    fn tag(&self) -> u8 {
        match self {
            Value::Integer(_) => tag::INTEGER,
            Value::Boolean(_) => tag::BOOLEAN,
            Value::Enum(_) => tag::ENUM,
            Value::Text(t, _) => *t,
        }
    }
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Value::Integer(v) | Value::Enum(v) => out.extend_from_slice(&v.to_be_bytes()),
            Value::Boolean(b) => out.push(if *b { 1 } else { 0 }),
            Value::Text(_, s) => out.extend_from_slice(s.as_bytes()),
        }
    }
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(_, s) => Some(s),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Integer(v) | Value::Enum(v) => Some(*v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub name: String,
    pub values: Vec<Value>,
}

impl Attr {
    pub fn new(name: &str, value: Value) -> Self {
        Self {
            name: name.to_string(),
            values: vec![value],
        }
    }
    pub fn multi(name: &str, values: Vec<Value>) -> Self {
        Self {
            name: name.to_string(),
            values,
        }
    }
    pub fn uri(name: &str, v: &str) -> Self {
        Self::new(name, Value::Text(tag::URI, v.to_string()))
    }
    pub fn keyword(name: &str, v: &str) -> Self {
        Self::new(name, Value::Text(tag::KEYWORD, v.to_string()))
    }
    pub fn text(name: &str, v: &str) -> Self {
        Self::new(name, Value::Text(tag::TEXT, v.to_string()))
    }
    pub fn name(name: &str, v: &str) -> Self {
        Self::new(name, Value::Text(tag::NAME, v.to_string()))
    }
    pub fn integer(name: &str, v: i32) -> Self {
        Self::new(name, Value::Integer(v))
    }
    pub fn enumeration(name: &str, v: i32) -> Self {
        Self::new(name, Value::Enum(v))
    }
    pub fn boolean(name: &str, v: bool) -> Self {
        Self::new(name, Value::Boolean(v))
    }
}

fn base_operation_attrs(printer_uri: &str) -> Vec<Attr> {
    vec![
        Attr::new(
            "attributes-charset",
            Value::Text(tag::CHARSET, "utf-8".to_string()),
        ),
        Attr::new(
            "attributes-natural-language",
            Value::Text(tag::LANGUAGE, "en".to_string()),
        ),
        Attr::uri("printer-uri", printer_uri),
    ]
}

fn write_attr(out: &mut Vec<u8>, attr: &Attr) {
    for (i, value) in attr.values.iter().enumerate() {
        out.push(value.tag());
        if i == 0 {
            out.extend_from_slice(&(attr.name.len() as u16).to_be_bytes());
            out.extend_from_slice(attr.name.as_bytes());
        } else {
            // Additional values: zero-length name (RFC 8010 section 3.1.4).
            out.extend_from_slice(&0u16.to_be_bytes());
        }
        let mut vb = Vec::new();
        value.encode(&mut vb);
        out.extend_from_slice(&(vb.len() as u16).to_be_bytes());
        out.extend_from_slice(&vb);
    }
}

fn write_header(out: &mut Vec<u8>, code: u16, request_id: u32) {
    out.extend_from_slice(&0x0101u16.to_be_bytes()); // version 1.1
    out.extend_from_slice(&code.to_be_bytes());
    out.extend_from_slice(&request_id.to_be_bytes());
}

/// Encode a request: operation group, optional job group, end tag,
/// then the document bytes (Print-Job only).
pub fn encode_request(
    operation: u16,
    request_id: u32,
    printer_uri: &str,
    extra_operation_attrs: &[Attr],
    job_attrs: &[Attr],
    document: Option<&[u8]>,
) -> Vec<u8> {
    let mut out = Vec::new();
    write_header(&mut out, operation, request_id);
    out.push(tag::GROUP_OPERATION);
    for a in base_operation_attrs(printer_uri) {
        write_attr(&mut out, &a);
    }
    for a in extra_operation_attrs {
        write_attr(&mut out, a);
    }
    if !job_attrs.is_empty() {
        out.push(tag::GROUP_JOB);
        for a in job_attrs {
            write_attr(&mut out, a);
        }
    }
    out.push(tag::END_OF_ATTRIBUTES);
    if let Some(doc) = document {
        out.extend_from_slice(doc);
    }
    out
}

/// Encode a response (used by printers, and by our mock in tests).
pub fn encode_response(status: u16, request_id: u32, groups: &[(u8, Vec<Attr>)]) -> Vec<u8> {
    let mut out = Vec::new();
    write_header(&mut out, status, request_id);
    out.push(tag::GROUP_OPERATION);
    for a in base_operation_attrs("ipp://localhost/ipp/print") {
        write_attr(&mut out, &a);
    }
    for (group_tag, attrs) in groups {
        out.push(*group_tag);
        for a in attrs {
            write_attr(&mut out, a);
        }
    }
    out.push(tag::END_OF_ATTRIBUTES);
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct IppMessage {
    pub version: (u8, u8),
    /// Operation id (requests) or status code (responses).
    pub code: u16,
    pub request_id: u32,
    /// (group delimiter tag, attributes) in wire order.
    pub groups: Vec<(u8, Vec<Attr>)>,
    /// Bytes after the end-of-attributes tag (document data).
    pub data: Vec<u8>,
}

impl IppMessage {
    pub fn attrs_in(&self, group: u8) -> impl Iterator<Item = &Attr> {
        self.groups
            .iter()
            .filter(move |(g, _)| *g == group)
            .flat_map(|(_, attrs)| attrs.iter())
    }
    pub fn find(&self, group: u8, name: &str) -> Option<&Attr> {
        self.attrs_in(group).find(|a| a.name == name)
    }
    pub fn texts(&self, group: u8, name: &str) -> Vec<String> {
        match self.find(group, name) {
            Some(a) => a
                .values
                .iter()
                .filter_map(|v| v.as_text().map(|s| s.to_string()))
                .collect(),
            None => Vec::new(),
        }
    }
    pub fn int(&self, group: u8, name: &str) -> Option<i32> {
        self.find(group, name)
            .and_then(|a| a.values.first())
            .and_then(|v| v.as_int())
    }
    pub fn bool(&self, group: u8, name: &str) -> Option<bool> {
        self.find(group, name)
            .and_then(|a| a.values.first())
            .and_then(|v| match v {
                Value::Boolean(b) => Some(*b),
                _ => None,
            })
    }
    /// How many times an attribute name appears across all groups
    /// (the document-format duplication check from CUPS issue #1457).
    pub fn count_attr(&self, name: &str) -> usize {
        self.groups
            .iter()
            .flat_map(|(_, attrs)| attrs.iter())
            .filter(|a| a.name == name)
            .count()
    }
}

fn parse_value(vtag: u8, raw: &[u8]) -> Value {
    match vtag {
        tag::INTEGER if raw.len() == 4 => {
            Value::Integer(i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
        }
        tag::ENUM if raw.len() == 4 => {
            Value::Enum(i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
        }
        tag::BOOLEAN if raw.len() == 1 => Value::Boolean(raw[0] != 0),
        t => Value::Text(t, String::from_utf8_lossy(raw).to_string()),
    }
}

/// Parse a request or response. Never panics on hostile input; malformed
/// messages return Err with the offset that failed.
pub fn parse_message(bytes: &[u8]) -> Result<IppMessage, String> {
    if bytes.len() < 9 {
        return Err("message too short".to_string());
    }
    let version = (bytes[0], bytes[1]);
    let code = u16::from_be_bytes([bytes[2], bytes[3]]);
    let request_id = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let mut pos = 8usize;
    let mut groups: Vec<(u8, Vec<Attr>)> = Vec::new();
    let mut current_group: Option<u8> = None;
    let mut last_attr: Option<usize> = None; // index into current (last) group's attrs
    loop {
        if pos >= bytes.len() {
            return Err(format!("unterminated message at {pos}"));
        }
        let b = bytes[pos];
        if b == tag::END_OF_ATTRIBUTES {
            pos += 1;
            break;
        }
        // RFC 8010 section 3.5.1: delimiter tags are 0x00..=0x0F (0x03 is handled above).
        if (0x01..=0x0F).contains(&b) {
            current_group = Some(b);
            groups.push((b, Vec::new()));
            last_attr = None;
            pos += 1;
            continue;
        }
        // A value tag.
        if current_group.is_none() {
            return Err(format!("attribute value before any group at {pos}"));
        };
        let vtag = b;
        if pos + 3 > bytes.len() {
            return Err(format!("truncated attribute header at {pos}"));
        }
        let name_len = u16::from_be_bytes([bytes[pos + 1], bytes[pos + 2]]) as usize;
        let name_start = pos + 3;
        let value_len_at = name_start + name_len;
        if value_len_at + 2 > bytes.len() {
            return Err(format!("truncated attribute name at {pos}"));
        }
        let name = String::from_utf8_lossy(&bytes[name_start..value_len_at]).to_string();
        let value_len = u16::from_be_bytes([bytes[value_len_at], bytes[value_len_at + 1]]) as usize;
        let value_start = value_len_at + 2;
        let value_end = value_start + value_len;
        if value_end > bytes.len() {
            return Err(format!("truncated attribute value at {pos}"));
        }
        let value = parse_value(vtag, &bytes[value_start..value_end]);
        // Always append into the most recently opened group (`groups.last_mut()`),
        // so multiple groups with the same delimiter tag never panic on `last_attr`.
        let group = groups.last_mut().ok_or("group vanished")?;
        if name_len == 0 {
            // Continuation value for the previous attribute.
            let Some(idx) = last_attr else {
                return Err("continuation value with no previous attribute".to_string());
            };
            group.1[idx].values.push(value);
        } else {
            group.1.push(Attr {
                name,
                values: vec![value],
            });
            last_attr = Some(group.1.len() - 1);
        }
        pos = value_end;
    }
    Ok(IppMessage {
        version,
        code,
        request_id,
        groups,
        data: bytes[pos..].to_vec(),
    })
}

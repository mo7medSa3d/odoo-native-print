use crate::mono::MonoBitmap;

/// ZPL ^GFA graphic field: ^GFA,<total_bytes>,<total_bytes>,<bytes_per_row>,<hex data>
/// Hex is uppercase ASCII, one nibble pair per byte, no compression (correct
/// first; compression is an optimisation, not a correctness feature).
pub fn zpl_graphic_field(bm: &MonoBitmap) -> String {
    let total = bm.data.len();
    let bpr = bm.bytes_per_row();
    let mut hex = String::with_capacity(total * 2);
    for b in &bm.data {
        hex.push_str(&format!("{b:02X}"));
    }
    format!("^GFA,{total},{total},{bpr},{hex}")
}

/// Complete ZPL II label format: `^FO0,0` (Field Origin) must precede `^GFA`
/// and end with `^FS` (Field Separator) before `^XZ`.
pub fn zpl_label(bm: &MonoBitmap) -> String {
    zpl_label_with_options(bm, 0, 0, 1)
}

/// Complete ZPL II label with print width (`^PW`), label length (`^LL`),
/// label home offset (`^LH`), field origin (`^FO0,0`), graphic field (`^GFA`),
/// field separator (`^FS`), and print quantity (`^PQ`).
pub fn zpl_label_with_options(
    bm: &MonoBitmap,
    offset_x_dots: i32,
    offset_y_dots: i32,
    copies: u16,
) -> String {
    let copies = copies.max(1);
    let lh_x = offset_x_dots.max(0);
    let lh_y = offset_y_dots.max(0);
    format!(
        "^XA^PW{}^LL{}^LH{lh_x},{lh_y}^FO0,0{}^FS^PQ{copies},0,1,Y^XZ",
        bm.width_dots.max(2),
        bm.height_dots.max(1),
        zpl_graphic_field(bm)
    )
}

/// ZPL host status: the `~HS` command (a preparser command,
/// processed immediately) and the SGD `device.host_status` getvar
/// return the same three comma-separated strings. `~HS` frames each
/// line with STX/ETX; the SGD form omits them and ends lines with
/// CR/LF. The parser accepts both framings and tolerates short or
/// dummy-valued fields: printers return dummy values in several
/// sub-fields, and firmware families differ in trailing fields.
///
/// String 1: `aaa,b,c,dddd,eee,f,g,h,iii,j,k,l`
///   b = paper-out flag, c = pause flag, dddd = label length (dots),
///   eee = formats in receive buffer, f = buffer-full flag,
///   h = partial-format flag.
/// String 2: `mmm,n,o,p,q,r,s,t,uuuuuuuu,v,www`
///   o = head-up flag, p = ribbon-out flag, t = label-waiting flag,
///   uuuuuuuu = labels remaining in batch.
/// String 3: `xxxx,y` — password / static-RAM flag (not health).
///
/// Caveat (Zebra SGD docs): the `device.host_status` getvar sends NO
/// response at all while the printer is in MEDIA OUT, RIBBON OUT,
/// HEAD OPEN, REWINDER FULL, or HEAD OVER-TEMPERATURE. Absence of a
/// response is therefore evidence of a fault state or an
/// unreachable printer — never evidence of readiness.
pub const ZPL_HOST_STATUS_QUERY: &str = "~HS";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ZplHostStatus {
    pub paper_out: bool,
    pub paused: bool,
    pub head_up: bool,
    pub ribbon_out: bool,
    pub buffer_full: bool,
    pub partial_format: bool,
    pub label_waiting: bool,
    pub label_length_dots: Option<u32>,
    pub formats_in_buffer: Option<u32>,
    pub labels_remaining: Option<u32>,
    /// The three raw response lines, kept for diagnostics.
    pub raw_lines: Vec<String>,
}

impl ZplHostStatus {
    pub fn blocks_printing(&self) -> bool {
        self.paper_out || self.paused || self.head_up || self.ribbon_out
    }
}

fn flag(field: Option<&str>) -> bool {
    field.is_some_and(|f| f.trim() == "1")
}

fn number(field: Option<&str>) -> Option<u32> {
    field.and_then(|f| f.trim().parse::<u32>().ok())
}

/// Parse a `~HS` / `device.host_status` response. Returns `None`
/// when no usable status line is present (empty response, garbage):
/// the caller must treat that as unknown, not ready.
pub fn parse_host_status(response: &str) -> Option<ZplHostStatus> {
    let cleaned: String = response
        .chars()
        .filter(|c| *c != '\u{2}' && *c != '\u{3}')
        .collect();
    let lines: Vec<String> = cleaned
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    let fields = |idx: usize| -> Vec<&str> {
        lines
            .get(idx)
            .map(|l| l.split(',').collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let s1 = fields(0);
    let s2 = fields(1);
    fn get<'a>(v: &[&'a str], i: usize) -> Option<&'a str> {
        v.get(i).copied()
    }
    Some(ZplHostStatus {
        paper_out: flag(get(&s1, 1)),
        paused: flag(get(&s1, 2)),
        label_length_dots: number(get(&s1, 3)),
        formats_in_buffer: number(get(&s1, 4)),
        buffer_full: flag(get(&s1, 5)),
        partial_format: flag(get(&s1, 7)),
        head_up: flag(get(&s2, 2)),
        ribbon_out: flag(get(&s2, 3)),
        label_waiting: flag(get(&s2, 7)),
        labels_remaining: number(get(&s2, 8)),
        raw_lines: lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_status_parses_both_framings() {
        // ~HS framing: STX at the start of each line, ETX at the end.
        let framed = "\u{2}000,1,0,1200,000,0,0,0,000,0,0,0\u{3}\n\u{2}000,0,1,1,0,0,0,0,00000005,1,000\u{3}\n\u{2}0000,0\u{3}";
        let st = parse_host_status(framed).unwrap();
        assert!(st.paper_out);
        assert!(!st.paused);
        assert!(st.head_up);
        assert!(st.ribbon_out);
        assert_eq!(st.label_length_dots, Some(1200));
        assert_eq!(st.labels_remaining, Some(5));
        assert!(st.blocks_printing());
        // SGD framing: same content, no STX/ETX, CR/LF line ends.
        let sgd =
            "000,0,1,800,001,1,0,0,000,0,0,0\r\n000,0,0,0,1,0,0,1,00000000,1,000\r\n0000,1\r\n";
        let st = parse_host_status(sgd).unwrap();
        assert!(!st.paper_out);
        assert!(st.paused);
        assert!(st.buffer_full);
        assert!(st.label_waiting);
        assert!(!st.head_up && !st.ribbon_out);
        assert_eq!(st.label_length_dots, Some(800));
        // Ready printer: all flags clear.
        let ready = parse_host_status(
            "000,0,0,1200,000,0,0,0,000,0,0,0\n000,0,0,0,0,0,0,0,00000001,1,000\n0000,0",
        )
        .unwrap();
        assert!(!ready.blocks_printing());
        assert!(!ready.paper_out && !ready.paused && !ready.head_up && !ready.ribbon_out);
    }

    #[test]
    fn host_status_tolerates_short_and_empty_responses() {
        assert_eq!(parse_host_status(""), None);
        assert_eq!(parse_host_status("\n\n"), None);
        // One short line: missing fields read as clear, never panic.
        let st = parse_host_status("000,1").unwrap();
        assert!(st.paper_out);
        assert!(!st.head_up);
        assert_eq!(st.label_length_dots, None);
    }

    #[test]
    fn gfa_header_and_hex() {
        let bm = MonoBitmap::from_packed(8, 1, vec![0b1010_1010]).unwrap();
        assert_eq!(zpl_graphic_field(&bm), "^GFA,1,1,1,AA");
        let label = zpl_label(&bm);
        assert!(label.starts_with("^XA") && label.ends_with("^XZ"));
        assert!(label.contains("^FO0,0^GFA,1,1,1,AA^FS"));
    }

    #[test]
    fn label_options_include_dimensions_and_copies() {
        let bm = MonoBitmap::from_packed(16, 4, vec![0xFF; 8]).unwrap();
        let label = zpl_label_with_options(&bm, 12, 8, 3);
        assert!(label.contains("^PW16^LL4^LH12,8^FO0,0^GFA,8,8,2,"));
        assert!(label.ends_with("^FS^PQ3,0,1,Y^XZ"));
    }
}

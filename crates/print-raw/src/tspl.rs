use crate::mono::MonoBitmap;

/// TSPL (TSC label printers) program builder. Structure of a label job:
/// SIZE, GAP, CLS, BITMAP commands, PRINT.
///
/// TSPL2 BITMAP command specification (TSC TSPL/TSPL2 Programming Manual):
/// `BITMAP X,Y,width,height,mode,bitmap data`
/// In TSPL2 mode 0 (OVERWRITE), bit `0` prints a black dot and bit `1` leaves
/// paper white (the opposite of ESC/POS and ZPL, where `1` = black dot).
/// Since `MonoBitmap` stores `1 = black dot`, we bitwise-invert (`!b`) each byte
/// when writing the TSPL `BITMAP` payload.
pub struct TsplLabel {
    buf: Vec<u8>,
}

impl TsplLabel {
    /// width_mm x height_mm label, gap_mm between labels.
    pub fn new(width_mm: u32, height_mm: u32, gap_mm: u32) -> Self {
        let mut buf = Vec::new();
        buf.extend_from_slice(format!("SIZE {width_mm} mm,{height_mm} mm\r\n").as_bytes());
        buf.extend_from_slice(format!("GAP {gap_mm} mm,0 mm\r\n").as_bytes());
        buf.extend_from_slice(b"CLS\r\n");
        Self { buf }
    }
    /// BITMAP x,y,width(bytes per row),height,mode(0 = overwrite),<inverted data>
    pub fn bitmap(mut self, x: u32, y: u32, bm: &MonoBitmap) -> Self {
        self.buf.extend_from_slice(
            format!(
                "BITMAP {x},{y},{},{},0,",
                bm.bytes_per_row(),
                bm.height_dots
            )
            .as_bytes(),
        );
        for &b in &bm.data {
            self.buf.push(!b);
        }
        self.buf.extend_from_slice(b"\r\n");
        self
    }
    /// TEXT x,y,"font",rotation,x-mult,y-mult,"content"
    pub fn text(mut self, x: u32, y: u32, content: &str) -> Self {
        // Strip control bytes: an embedded CR/LF would break out of
        // the TEXT command into arbitrary TSPL commands (the ESC/POS
        // builder strips them for the same reason). Quotes are folded
        // so the string literal cannot terminate early.
        let safe: String = content
            .chars()
            .filter(|c| !c.is_control())
            .map(|c| if c == '"' { '\'' } else { c })
            .collect();
        self.buf
            .extend_from_slice(format!("TEXT {x},{y},\"3\",0,1,1,\"{safe}\"\r\n").as_bytes());
        self
    }
    pub fn print(mut self, sets: u32, copies: u32) -> Vec<u8> {
        self.buf
            .extend_from_slice(format!("PRINT {sets},{copies}\r\n").as_bytes());
        self.buf
    }
}

/// TSPL status polling: `ESC !?` (0x1B 0x21 0x3F) returns one
/// status byte. Send it ONLY on a bidirectional transport (TCP 9100
/// or serial) with no print data in flight: the response has no
/// framing, so interleaving it with a label stream corrupts both.
/// A Windows RAW spooler queue is write-only and cannot read the
/// reply — status there stays spooler-reported, never simulated.
pub const TSPL_STATUS_QUERY: [u8; 3] = [0x1B, b'!', b'?'];

/// Decoded `ESC !?` status byte (TSC TSPL/TSPL2 Programming Manual;
/// Zebra TSPL guide value table — the two agree: the byte is a bit
/// field, and the documented hex values are its combinations).
/// Bit 6 (cover opened) is marked optional in the TSC manual and is
/// model-dependent: treat it as cover-open evidence, not proof, on
/// models that do not document it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TsplStatus {
    pub raw: u8,
    pub head_open: bool,
    pub paper_jam: bool,
    pub paper_out: bool,
    pub ribbon_out: bool,
    pub paused: bool,
    pub printing: bool,
    pub cover_open: bool,
    pub other_error: bool,
}

impl TsplStatus {
    pub fn is_ready(&self) -> bool {
        self.raw == 0
    }
    pub fn blocks_printing(&self) -> bool {
        self.head_open
            || self.paper_jam
            || self.paper_out
            || self.ribbon_out
            || self.paused
            || self.cover_open
            || self.other_error
    }
}

pub fn decode_tspl_status(byte: u8) -> TsplStatus {
    TsplStatus {
        raw: byte,
        head_open: byte & 0x01 != 0,
        paper_jam: byte & 0x02 != 0,
        paper_out: byte & 0x04 != 0,
        ribbon_out: byte & 0x08 != 0,
        paused: byte & 0x10 != 0,
        printing: byte & 0x20 != 0,
        cover_open: byte & 0x40 != 0,
        other_error: byte & 0x80 != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tspl_status_byte_table() {
        // Zebra TSPL guide hex table, decoded as bit flags.
        // (byte, head, jam, paper, ribbon, pause, printing, other)
        type Case = (u8, bool, bool, bool, bool, bool, bool, bool);
        let cases: &[Case] = &[
            (0x00, false, false, false, false, false, false, false),
            (0x01, true, false, false, false, false, false, false),
            (0x02, false, true, false, false, false, false, false),
            (0x03, true, true, false, false, false, false, false),
            (0x04, false, false, true, false, false, false, false),
            (0x05, true, false, true, false, false, false, false),
            (0x08, false, false, false, true, false, false, false),
            (0x0A, false, true, false, true, false, false, false),
            (0x0C, false, false, true, true, false, false, false),
            (0x10, false, false, false, false, true, false, false),
            (0x20, false, false, false, false, false, true, false),
            (0x80, false, false, false, false, false, false, true),
        ];
        for &(byte, head, jam, paper, ribbon, pause, printing, other) in cases {
            let s = decode_tspl_status(byte);
            assert_eq!(s.head_open, head, "byte 0x{byte:02x}");
            assert_eq!(s.paper_jam, jam, "byte 0x{byte:02x}");
            assert_eq!(s.paper_out, paper, "byte 0x{byte:02x}");
            assert_eq!(s.ribbon_out, ribbon, "byte 0x{byte:02x}");
            assert_eq!(s.paused, pause, "byte 0x{byte:02x}");
            assert_eq!(s.printing, printing, "byte 0x{byte:02x}");
            assert_eq!(s.other_error, other, "byte 0x{byte:02x}");
        }
        assert!(decode_tspl_status(0x00).is_ready());
        assert!(!decode_tspl_status(0x00).blocks_printing());
        assert!(decode_tspl_status(0x04).blocks_printing());
        assert!(!decode_tspl_status(0x20).blocks_printing());
        assert_eq!(TSPL_STATUS_QUERY, [0x1B, b'!', b'?']);
    }

    #[test]
    fn text_strips_control_injection() {
        let out = TsplLabel::new(50, 30, 3)
            .text(1, 1, "evil\"\r\nPRINT 99\u{7}")
            .print(1, 1);
        let text = String::from_utf8_lossy(&out);
        // The words stay as inert quoted TEXT data; the CR/LF that
        // would have turned them into a command must be gone.
        assert!(
            !text.contains("\r\nPRINT 99"),
            "CR/LF injection must be stripped: {text}"
        );
        assert_eq!(
            text.matches("\r\nPRINT ").count(),
            1,
            "only the real PRINT command remains"
        );
    }

    #[test]
    fn label_program_shape() {
        let bm = MonoBitmap::from_packed(16, 2, vec![0xFF, 0x00, 0x00, 0xFF]).unwrap();
        let out = TsplLabel::new(100, 150, 3).bitmap(0, 0, &bm).print(1, 2);
        let head = String::from_utf8_lossy(&out[..out.len() - 14]).to_string();
        assert!(head.starts_with("SIZE 100 mm,150 mm\r\nGAP 3 mm,0 mm\r\nCLS\r\nBITMAP 0,0,2,2,0,"));
        assert!(out.ends_with(b"PRINT 1,2\r\n"));
        // TSPL 0=black / 1=white: inverted from MonoBitmap's 1=black ([0xFF, 0x00, 0x00, 0xFF] -> [0x00, 0xFF, 0xFF, 0x00]).
        assert!(out.windows(4).any(|w| w == [0x00, 0xFF, 0xFF, 0x00]));
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
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

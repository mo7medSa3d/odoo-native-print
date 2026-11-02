use crate::mono::MonoBitmap;

/// TSPL (TSC label printers) program builder. Structure of a label job:
/// SIZE, GAP, CLS, BITMAP commands, PRINT. Bitmap data is binary,
/// row-major, 1 = black, matching MonoBitmap's packing.
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
    /// BITMAP x,y,width(bytes per row),height(mode 0 = overwrite),data
    pub fn bitmap(mut self, x: u32, y: u32, bm: &MonoBitmap) -> Self {
        self.buf.extend_from_slice(
            format!(
                "BITMAP {x},{y},{},{},0,",
                bm.bytes_per_row(),
                bm.height_dots
            )
            .as_bytes(),
        );
        self.buf.extend_from_slice(&bm.data);
        self.buf.extend_from_slice(b"\r\n");
        self
    }
    /// TEXT x,y,"font",rotation,x-mult,y-mult,"content"
    pub fn text(mut self, x: u32, y: u32, content: &str) -> Self {
        let safe = content.replace('"', "'");
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
    fn label_program_shape() {
        let bm = MonoBitmap::from_packed(16, 2, vec![0xFF, 0x00, 0x00, 0xFF]).unwrap();
        let out = TsplLabel::new(100, 150, 3).bitmap(0, 0, &bm).print(1, 2);
        let head = String::from_utf8_lossy(&out[..out.len() - 14]).to_string();
        assert!(head.starts_with("SIZE 100 mm,150 mm\r\nGAP 3 mm,0 mm\r\nCLS\r\nBITMAP 0,0,2,2,0,"));
        assert!(out.ends_with(b"PRINT 1,2\r\n"));
        // Binary bitmap bytes are present verbatim after the BITMAP header.
        assert!(out.windows(4).any(|w| w == [0xFF, 0x00, 0x00, 0xFF]));
    }
}

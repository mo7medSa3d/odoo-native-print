use crate::mono::MonoBitmap;

/// ESC/POS command builder. Bytes only — transports live elsewhere.
/// Reference commands: GS v 0 (raster), GS V (cut), ESC p (drawer), LF/ESC d (feed).
///
/// Long receipt safety: `GS v 0` (`1D 76 30 m xL xH yL yH`) takes a 16-bit height (`y`),
/// and many thermal printer firmwares have a 4–64 KB receive/raster buffer limit per
/// `GS v 0` command. `raster()` therefore emits horizontal slices of at most
/// `MAX_RASTER_SLICE_HEIGHT` (512 dots, ~64 mm at 203 dpi) so arbitrarily long
/// receipts print without buffer overflow or u16 truncation. 512 is a
/// deliberately conservative band: cheap thermal clones have small
/// receive buffers (a 1024-row band at 576 dots is ~74 KB in a single
/// command), and halving the band costs nothing measurable.
pub const MAX_RASTER_SLICE_HEIGHT: u32 = 512;

pub struct EscPos {
    buf: Vec<u8>,
}

impl Default for EscPos {
    fn default() -> Self {
        Self::new()
    }
}

impl EscPos {
    pub fn new() -> Self {
        Self {
            buf: vec![0x1B, 0x40],
        }
    } // ESC @ init
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// GS v 0 raster image: 1D 76 30 m xL xH yL yH <data>
    /// m=0 normal density. x = bytes per row, y = height in dots (little-endian u16).
    /// Automatically chunks bitmaps taller than `MAX_RASTER_SLICE_HEIGHT` into
    /// consecutive `GS v 0` slices.
    pub fn raster(mut self, bm: &MonoBitmap) -> Self {
        let bpr = bm.bytes_per_row();
        let x = bpr as u16;
        let total_h = bm.height_dots;
        if total_h == 0 || bpr == 0 {
            return self;
        }
        let mut row_start = 0u32;
        while row_start < total_h {
            let slice_h = (total_h - row_start).min(MAX_RASTER_SLICE_HEIGHT);
            let y = slice_h as u16;
            let byte_start = (row_start as usize) * bpr;
            let byte_end = ((row_start + slice_h) as usize) * bpr;
            self.buf.extend_from_slice(&[0x1D, 0x76, 0x30, 0x00]);
            self.buf.extend_from_slice(&x.to_le_bytes());
            self.buf.extend_from_slice(&y.to_le_bytes());
            self.buf.extend_from_slice(&bm.data[byte_start..byte_end]);
            row_start += slice_h;
        }
        self
    }
    pub fn feed_lines(mut self, n: u8) -> Self {
        self.buf.extend_from_slice(&[0x1B, 0x64, n]);
        self
    }
    pub fn cut_full(mut self) -> Self {
        self.buf.extend_from_slice(&[0x1D, 0x56, 0x00]);
        self
    }
    pub fn cut_partial(mut self) -> Self {
        self.buf.extend_from_slice(&[0x1D, 0x56, 0x01]);
        self
    }
    /// ESC p m t1 t2 — cash drawer kick on pin 2 (m=0), 25ms/250ms typical.
    pub fn drawer_kick(mut self) -> Self {
        self.buf.extend_from_slice(&[0x1B, 0x70, 0x00, 0x19, 0xFA]);
        self
    }
    /// Write one text line, stripping raw ESC (0x1B), FS (0x1C), GS (0x1D),
    /// DLE (0x10), and NUL (0x00) control bytes so untrusted report text
    /// can never inject raw ESC/POS hardware commands (e.g. cash drawer kick).
    pub fn text_line(mut self, line: &str) -> Self {
        for &b in line.as_bytes() {
            if !matches!(b, 0x00 | 0x10 | 0x1B | 0x1C | 0x1D) {
                self.buf.push(b);
            }
        }
        self.buf.push(0x0A);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Test decoder: parse one or more GS v 0 blocks back into a single bitmap,
    /// proving the encoder round-trips (spec S7, mock-level evidence).
    fn decode_raster(stream: &[u8]) -> MonoBitmap {
        let mut pos = 0usize;
        let mut width_dots = 0u32;
        let mut height_dots = 0u32;
        let mut data = Vec::new();
        while let Some(rel) = stream[pos..]
            .windows(4)
            .position(|w| w == [0x1D, 0x76, 0x30, 0x00])
        {
            let hpos = pos + rel;
            let x = u16::from_le_bytes([stream[hpos + 4], stream[hpos + 5]]) as usize;
            let y = u16::from_le_bytes([stream[hpos + 6], stream[hpos + 7]]) as usize;
            let slice_start = hpos + 8;
            let slice_end = slice_start + x * y;
            width_dots = (x * 8) as u32;
            height_dots += y as u32;
            data.extend_from_slice(&stream[slice_start..slice_end]);
            pos = slice_end;
        }
        assert!(height_dots > 0, "no GS v 0 block found");
        MonoBitmap {
            width_dots,
            height_dots,
            data,
        }
    }
    #[test]
    fn raster_round_trip() {
        let gray: Vec<u8> = (0..16 * 4)
            .map(|i| if i % 3 == 0 { 0 } else { 255 })
            .collect();
        let bm = MonoBitmap::from_grayscale(&gray, 16, 4, 128).unwrap();
        let out = EscPos::new().raster(&bm).feed_lines(3).cut_partial();
        let decoded = decode_raster(out.bytes());
        assert_eq!(decoded.data, bm.data);
        assert_eq!(decoded.height_dots, 4);
        assert!(out.bytes().ends_with(&[0x1D, 0x56, 0x01]));
    }
    #[test]
    fn long_receipt_chunks_cleanly() {
        let bm = MonoBitmap::from_packed(16, 2500, vec![0xAA; 2 * 2500]).unwrap();
        let out = EscPos::new().raster(&bm).cut_partial();
        let decoded = decode_raster(out.bytes());
        assert_eq!(decoded.height_dots, 2500);
        assert_eq!(decoded.data, bm.data);
    }
    #[test]
    fn text_line_strips_esc_pos_control_injection() {
        let out = EscPos::new().text_line("ITEM 1\x1Bp\x00\x19\u{fa}\x1DV\x00");
        // Starts with ESC @ (2 bytes), then "ITEM 1", p, 0x19, 0xFA, V, LF — no injected ESC (0x1B) or GS (0x1D) after index 1.
        assert!(!out.bytes()[2..].contains(&0x1B));
        assert!(!out.bytes()[2..].contains(&0x1D));
        assert!(!out.bytes()[2..].contains(&0x00));
    }
    #[test]
    fn tcp_9100_mock_server_round_trip() {
        use std::io::Read;
        use std::net::{TcpListener, TcpStream};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).unwrap();
            buf
        });
        let bm = MonoBitmap::from_packed(16, 2, vec![0xAA, 0x55, 0xAA, 0x55]).unwrap();
        let payload = EscPos::new().raster(&bm).cut_full();
        let mut client = TcpStream::connect(addr).unwrap();
        use std::io::Write;
        client.write_all(payload.bytes()).unwrap();
        drop(client);
        let received = server.join().unwrap();
        assert_eq!(received, payload.bytes());
        assert_eq!(decode_raster(&received).data, bm.data);
    }

    #[test]
    fn receipt_80mm_width() {
        let bm = MonoBitmap::from_packed(576, 2, vec![0xFF; 72 * 2]).unwrap();
        let out = EscPos::new().raster(&bm);
        assert_eq!(&out.bytes()[2 + 4..2 + 6], &[72, 0]);
    }
}

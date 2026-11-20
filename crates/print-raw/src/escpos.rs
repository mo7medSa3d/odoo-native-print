use crate::mono::MonoBitmap;

/// ESC/POS command builder. Bytes only — transports live elsewhere.
/// Reference commands: GS v 0 (raster), GS V (cut), ESC p (drawer), LF/ESC d (feed).
///
/// Long receipt safety: `GS v 0` (`1D 76 30 m xL xH yL yH`) takes a 16-bit height (`y`),
/// and many thermal printer firmwares have a 4–64 KB receive/raster buffer limit per
/// `GS v 0` command. `raster()` therefore emits horizontal slices of at most
/// `MAX_RASTER_SLICE_HEIGHT` (256 dots, ~32 mm at 203 dpi) so arbitrarily long
/// receipts print without buffer overflow or u16 truncation. 256 is a
/// deliberately conservative band: budget thermal clones often have 16–32 KB
/// receive buffers (a 256-row band is 18 KB at 80 mm / 576 dots and ~50 KB
/// even at 210 mm / 1608 dots), while small bitmaps (<= 256 rows) remain a
/// single `GS v 0` command.
pub const MAX_RASTER_SLICE_HEIGHT: u32 = 256;

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
        let total_h = bm.height_dots;
        if total_h == 0 || bpr == 0 {
            return self;
        }
        // The GS v 0 header carries bytes-per-row as a u16: a wider
        // bitmap cannot be represented, and a truncated header would
        // desync the printer's byte stream. Likewise, a bitmap whose
        // data buffer disagrees with its dimensions (only possible
        // for a hand-constructed value — the constructors validate)
        // must not panic on the slice below. Emit nothing in both
        // cases (audit L6/L7). In-app widths are ~1200 dots max, so
        // neither guard can trigger on a real print path.
        let Some(expected) = (total_h as usize).checked_mul(bpr) else {
            debug_assert!(false, "bitmap not representable as ESC/POS raster");
            return self;
        };
        if bpr > u16::MAX as usize || bm.data.len() < expected {
            debug_assert!(false, "bitmap not representable as ESC/POS raster");
            return self;
        }
        let slices = (total_h as usize).div_ceil(MAX_RASTER_SLICE_HEIGHT as usize);
        self.buf
            .reserve(expected.saturating_add(slices.saturating_mul(8)));
        let x = bpr as u16;
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
    /// GS V 65 n — feed `n` motion units to cutter position, then full cut.
    pub fn cut_full_feed(mut self, n: u8) -> Self {
        self.buf.extend_from_slice(&[0x1D, 0x56, 0x41, n]);
        self
    }
    /// GS V 66 n — feed `n` motion units to cutter position, then partial cut.
    pub fn cut_partial_feed(mut self, n: u8) -> Self {
        self.buf.extend_from_slice(&[0x1D, 0x56, 0x42, n]);
        self
    }
    /// ESC p m t1 t2 — cash drawer kick on pin 2 (m=0), 50ms on / 500ms off.
    pub fn drawer_kick(self) -> Self {
        self.drawer_kick_pin(0x00, 0x19, 0xFA)
    }
    /// ESC p m t1 t2 — cash drawer kick on specified pin (0 = pin 2, 1 = pin 5).
    pub fn drawer_kick_pin(mut self, pin: u8, on_time_2ms: u8, off_time_2ms: u8) -> Self {
        self.buf
            .extend_from_slice(&[0x1B, 0x70, pin & 0x01, on_time_2ms, off_time_2ms]);
        self
    }
    /// Write one text line, stripping all ASCII control bytes (0x00..=0x1F,
    /// 0x7F) so untrusted report text can never inject raw ESC/POS hardware
    /// commands (e.g. cash drawer kick, CAN, FF, or extra line breaks).
    pub fn text_line(mut self, line: &str) -> Self {
        self.buf.reserve(line.len().saturating_add(1));
        for &b in line.as_bytes() {
            if b >= 0x20 && b != 0x7F {
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
        let out = EscPos::new().text_line("ITEM 1\x1Bp\x00\x18\x0C\r\n\u{fa}\x1DV\x7F");
        // Starts with ESC @ (2 bytes), ends with LF (0x0A); no control bytes in body.
        let body = &out.bytes()[2..out.bytes().len() - 1];
        assert!(body.iter().all(|&b| b >= 0x20 && b != 0x7F));
        assert_eq!(out.bytes().last(), Some(&0x0A));
    }
    #[test]
    fn cut_feed_and_drawer_commands() {
        let full = EscPos::new().cut_full_feed(4);
        assert!(full.bytes().ends_with(&[0x1D, 0x56, 0x41, 4]));
        let partial = EscPos::new().cut_partial_feed(5);
        assert!(partial.bytes().ends_with(&[0x1D, 0x56, 0x42, 5]));
        let drawer = EscPos::new().drawer_kick();
        assert!(drawer.bytes().ends_with(&[0x1B, 0x70, 0x00, 0x19, 0xFA]));
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

    #[test]
    fn banded_raster_respects_max_slice_height_and_preserves_small_bitmaps() {
        fn band_heights(stream: &[u8]) -> Vec<u16> {
            let mut heights = Vec::new();
            let mut pos = 0usize;
            while let Some(rel) = stream[pos..]
                .windows(4)
                .position(|w| w == [0x1D, 0x76, 0x30, 0x00])
            {
                let hpos = pos + rel;
                let x = u16::from_le_bytes([stream[hpos + 4], stream[hpos + 5]]) as usize;
                let y = u16::from_le_bytes([stream[hpos + 6], stream[hpos + 7]]);
                heights.push(y);
                pos = hpos + 8 + x * (y as usize);
            }
            heights
        }

        // Exactly MAX_RASTER_SLICE_HEIGHT rows stays a single GS v 0 command.
        let exact = MonoBitmap::from_packed(
            16,
            MAX_RASTER_SLICE_HEIGHT,
            vec![0x5A; 2 * MAX_RASTER_SLICE_HEIGHT as usize],
        )
        .unwrap();
        let exact_out = EscPos::new().raster(&exact);
        assert_eq!(
            band_heights(exact_out.bytes()),
            vec![MAX_RASTER_SLICE_HEIGHT as u16]
        );
        assert_eq!(decode_raster(exact_out.bytes()).data, exact.data);

        // One row taller splits into two bands (256 + 1).
        let over = MonoBitmap::from_packed(
            16,
            MAX_RASTER_SLICE_HEIGHT + 1,
            vec![0xA5; 2 * (MAX_RASTER_SLICE_HEIGHT as usize + 1)],
        )
        .unwrap();
        let over_out = EscPos::new().raster(&over);
        assert_eq!(
            band_heights(over_out.bytes()),
            vec![MAX_RASTER_SLICE_HEIGHT as u16, 1]
        );
        assert_eq!(decode_raster(over_out.bytes()).data, over.data);

        // 600 rows splits into 256 + 256 + 88 with exact byte round-trip.
        let tall_data: Vec<u8> = (0..(2 * 600)).map(|i| (i % 251) as u8).collect();
        let tall = MonoBitmap::from_packed(16, 600, tall_data).unwrap();
        let tall_out = EscPos::new().raster(&tall).cut_partial();
        assert_eq!(band_heights(tall_out.bytes()), vec![256, 256, 88]);
        let decoded = decode_raster(tall_out.bytes());
        assert_eq!(decoded.height_dots, 600);
        assert_eq!(decoded.data, tall.data);
    }
}

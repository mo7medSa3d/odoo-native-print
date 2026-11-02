use crate::mono::MonoBitmap;

/// ESC/POS command builder. Bytes only — transports live elsewhere.
/// Reference commands: GS v 0 (raster), GS V (cut), ESC p (drawer), LF/ESC d (feed).
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
    pub fn raster(mut self, bm: &MonoBitmap) -> Self {
        let x = bm.bytes_per_row() as u16;
        let y = bm.height_dots as u16;
        self.buf.extend_from_slice(&[0x1D, 0x76, 0x30, 0x00]);
        self.buf.extend_from_slice(&x.to_le_bytes());
        self.buf.extend_from_slice(&y.to_le_bytes());
        self.buf.extend_from_slice(&bm.data);
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
    pub fn text_line(mut self, line: &str) -> Self {
        self.buf.extend_from_slice(line.as_bytes());
        self.buf.push(0x0A);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Test decoder: parse a GS v 0 block back into a bitmap, proving the
    /// encoder round-trips (spec S7, mock-level evidence).
    fn decode_raster(stream: &[u8]) -> MonoBitmap {
        let pos = stream
            .windows(4)
            .position(|w| w == [0x1D, 0x76, 0x30, 0x00])
            .unwrap();
        let x = u16::from_le_bytes([stream[pos + 4], stream[pos + 5]]) as usize;
        let y = u16::from_le_bytes([stream[pos + 6], stream[pos + 7]]) as usize;
        let data = stream[pos + 8..pos + 8 + x * y].to_vec();
        MonoBitmap {
            width_dots: (x * 8) as u32,
            height_dots: y as u32,
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
    fn tcp_9100_mock_server_round_trip() {
        use std::io::Read;
        use std::net::{TcpListener, TcpStream};
        // Mock printer on an ephemeral port (the test never touches 9100
        // itself, so it cannot clash with a real printer service).
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
        drop(client); // close = end of job, as with real 9100 printers
        let received = server.join().unwrap();
        assert_eq!(received, payload.bytes());
        assert_eq!(decode_raster(&received).data, bm.data);
    }

    #[test]
    fn receipt_80mm_width() {
        // 80mm at 203 dpi = 576 dots printable on common thermal heads.
        let bm = MonoBitmap::from_packed(576, 2, vec![0xFF; 72 * 2]).unwrap();
        let out = EscPos::new().raster(&bm);
        // xL xH = 72, 0
        assert_eq!(&out.bytes()[2 + 4..2 + 6], &[72, 0]);
    }
}

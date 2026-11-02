use print_raw::MonoBitmap;

/// A grayscale drawing canvas (0 = black, 255 = white) at printer dots.
/// Used for the thermal calibration page and for composing raster receipts.
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            px: vec![255u8; (width as usize) * (height as usize)],
        }
    }
    fn idx(&self, x: u32, y: u32) -> usize {
        (y as usize) * (self.width as usize) + (x as usize)
    }
    pub fn set(&mut self, x: u32, y: u32) {
        if x < self.width && y < self.height {
            let i = self.idx(x, y);
            self.px[i] = 0;
        }
    }
    pub fn hline(&mut self, x0: u32, x1: u32, y: u32, thickness: u32) {
        for t in 0..thickness {
            for x in x0..=x1.min(self.width.saturating_sub(1)) {
                self.set(x, y + t);
            }
        }
    }
    pub fn vline(&mut self, x: u32, y0: u32, y1: u32, thickness: u32) {
        for t in 0..thickness {
            for y in y0..=y1.min(self.height.saturating_sub(1)) {
                self.set(x + t, y);
            }
        }
    }
    pub fn rect_outline(&mut self, x: u32, y: u32, w: u32, h: u32, thickness: u32) {
        self.hline(x, x + w - 1, y, thickness);
        self.hline(x, x + w - 1, y + h - 1, thickness);
        self.vline(x, y, y + h - 1, thickness);
        self.vline(x + w - 1, y, y + h - 1, thickness);
    }
    pub fn to_mono(&self, threshold: u8) -> MonoBitmap {
        MonoBitmap::from_grayscale(&self.px, self.width, self.height, threshold)
            .expect("canvas always produces a valid bitmap")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rect_corners_black_inside_white() {
        let mut c = Canvas::new(40, 30);
        c.rect_outline(5, 5, 20, 10, 1);
        let m = c.to_mono(128);
        assert!(m.pixel_is_black(5, 5));
        assert!(m.pixel_is_black(24, 14));
        assert!(!m.pixel_is_black(15, 10));
        assert!(!m.pixel_is_black(0, 0));
    }
}

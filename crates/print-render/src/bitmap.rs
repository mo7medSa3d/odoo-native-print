use print_raw::MonoBitmap;

/// A grayscale drawing canvas (0 = black, 255 = white) at printer dots.
/// Used for the thermal calibration page and for composing raster receipts.
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub px: Vec<u8>,
}

/// Split a canvas height into consecutive row chunks of at most
/// `rows_per_page` rows: the pagination plan for printing a tall
/// image (a POS receipt) across several sheets. Returns
/// `(start_row, row_count)` pairs covering the full height in
/// order; an empty height or a non-positive page size yields no
/// chunks (callers treat that as "nothing to print", never as a
/// one-page fallback that would rescale the whole strip).
pub fn page_row_chunks(height: u32, rows_per_page: u32) -> Vec<(u32, u32)> {
    if height == 0 || rows_per_page == 0 {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < height {
        let rows = rows_per_page.min(height - start);
        chunks.push((start, rows));
        start += rows;
    }
    chunks
}

/// Extract one row chunk of a canvas as its own canvas (rows are
/// copied; the chunk keeps the source width). Out-of-range chunks
/// clamp to the canvas bounds.
pub fn slice_canvas_rows(canvas: &Canvas, start_row: u32, rows: u32) -> Canvas {
    let width = canvas.width as usize;
    let start = (start_row.min(canvas.height)) as usize;
    let end = ((start_row.saturating_add(rows)).min(canvas.height)) as usize;
    let px = if width == 0 || end <= start {
        Vec::new()
    } else {
        canvas.px[start * width..end * width].to_vec()
    };
    Canvas {
        width: canvas.width,
        height: (end - start) as u32,
        px,
    }
}

/// Downscale a mono bitmap so it fits inside `max_w` x `max_h`
/// dots, preserving aspect ratio. Never upscales: a bitmap that
/// already fits is returned unchanged. The result width is aligned
/// down to a multiple of 8 dots, as packed bitmap formats require.
/// This is the guarantee that rendered receipts and labels never
/// exceed the printer's physical media width.
pub fn fit_mono_to_box(bm: &MonoBitmap, max_w: u32, max_h: u32) -> MonoBitmap {
    if bm.width_dots <= max_w && bm.height_dots <= max_h {
        return bm.clone();
    }
    if bm.width_dots == 0 || bm.height_dots == 0 || max_w < 8 || max_h == 0 {
        return bm.clone();
    }
    let scale = (max_w as f64 / bm.width_dots as f64)
        .min(max_h as f64 / bm.height_dots as f64)
        .min(1.0);
    let target_w = (((bm.width_dots as f64 * scale).floor() as u32) / 8 * 8).max(8);
    let target_h = ((bm.height_dots as f64 * scale).floor() as u32).max(1);
    let row_bytes = bm.bytes_per_row();
    let mut gray = vec![255u8; (bm.width_dots * bm.height_dots) as usize];
    for y in 0..bm.height_dots {
        for x in 0..bm.width_dots {
            let byte = bm.data[y as usize * row_bytes + (x as usize / 8)];
            if (byte >> (7 - (x % 8))) & 1 == 1 {
                gray[(y * bm.width_dots + x) as usize] = 0;
            }
        }
    }
    let mut out = vec![255u8; (target_w * target_h) as usize];
    for y in 0..target_h {
        let sy = ((y as f64 / scale).floor() as u32).min(bm.height_dots - 1);
        for x in 0..target_w {
            let sx = ((x as f64 / scale).floor() as u32).min(bm.width_dots - 1);
            out[(y * target_w + x) as usize] = gray[(sy * bm.width_dots + sx) as usize];
        }
    }
    MonoBitmap::from_grayscale(&out, target_w, target_h, 128)
        .expect("resampled buffer matches its dimensions")
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
        // Zero width/height would underflow the `w - 1` / `h - 1`
        // arithmetic below (debug panic, release wrap) — a degenerate
        // outline is simply nothing to draw (audit L9).
        if w == 0 || h == 0 {
            return;
        }
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
    fn row_chunks_cover_the_strip_exactly() {
        assert_eq!(page_row_chunks(0, 100), Vec::new());
        assert_eq!(page_row_chunks(100, 0), Vec::new());
        assert_eq!(
            page_row_chunks(250, 100),
            vec![(0, 100), (100, 100), (200, 50)]
        );
        assert_eq!(page_row_chunks(100, 100), vec![(0, 100)]);
        // Slicing returns the chunk's own rows, in order.
        let mut c = Canvas::new(4, 6);
        for y in 0..6 {
            for x in 0..4 {
                c.px[(y * 4 + x) as usize] = (y * 10) as u8;
            }
        }
        let mid = slice_canvas_rows(&c, 2, 3);
        assert_eq!((mid.width, mid.height), (4, 3));
        assert_eq!(mid.px[0], 20);
        assert_eq!(mid.px[11], 40);
        let tail = slice_canvas_rows(&c, 5, 100);
        assert_eq!(tail.height, 1);
    }

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

#[cfg(test)]
mod fit_tests {
    use super::*;

    fn solid(w: u32, h: u32) -> MonoBitmap {
        MonoBitmap::from_grayscale(&vec![0u8; (w * h) as usize], w, h, 128).unwrap()
    }

    #[test]
    fn fit_downscales_to_box_keeping_aspect() {
        let bm = solid(1600, 800);
        let out = fit_mono_to_box(&bm, 800, 800);
        assert_eq!((out.width_dots, out.height_dots), (800, 400));
    }

    #[test]
    fn fit_to_roll_width_aligns_to_8() {
        let bm = solid(1000, 500);
        let out = fit_mono_to_box(&bm, 700, u32::MAX);
        assert_eq!(out.width_dots, 696); // largest multiple of 8 <= 700
        assert!(out.height_dots <= 352);
    }

    #[test]
    fn fit_never_upscales() {
        let bm = solid(384, 200);
        let out = fit_mono_to_box(&bm, 576, 1000);
        assert_eq!((out.width_dots, out.height_dots), (384, 200));
        assert_eq!(out.data, bm.data);
    }
}

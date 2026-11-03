//! Platform-independent half of the GDI backend: turning a rendered
//! page into a Windows DIB (device-independent bitmap) byte layout,
//! plus the page-fit math. Everything here is pure data construction,
//! so it is compiled and unit-tested on every platform; only the DC
//! calls that consume it live behind `cfg(windows)` in `win.rs`.
//!
//! DIB facts used (Win32 BITMAPINFOHEADER, biSize = 40):
//! - Negative `biHeight` = top-down rows, matching PDFium's output
//!   order, so no row flipping is ever needed.
//! - Every pixel row is padded to a 4-byte boundary.
//! - 32 bpp pixels are B,G,R,A — exactly PDFium's BGRA output.
//! - 8 bpp needs a 256-entry grayscale palette; 1 bpp a 2-entry one.

use print_render::{RenderMode, RenderedPage};

/// A fully laid-out DIB: header bytes, palette bytes (possibly empty),
/// and pixel bits, ready for `SetDIBitsToDevice` / `StretchDIBits`
/// (which take a BITMAPINFO* = header followed by palette).
pub struct DibImage {
    pub width: u32,
    pub height: u32,
    /// BITMAPINFOHEADER (40 bytes, little-endian).
    pub header: [u8; 40],
    /// Palette entries, 4 bytes each (B,G,R,0). Concatenate directly
    /// after `header` to form the BITMAPINFO block.
    pub palette: Vec<u8>,
    /// Pixel rows, each padded to a 4-byte boundary, top-down.
    pub bits: Vec<u8>,
    pub bit_count: u16,
}

impl DibImage {
    /// BITMAPINFO block (header + palette) as one buffer.
    pub fn bitmap_info(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.header.len() + self.palette.len());
        v.extend_from_slice(&self.header);
        v.extend_from_slice(&self.palette);
        v
    }
}

/// Bytes per DIB row for a width and bit depth (4-byte aligned).
pub fn dib_row_bytes(width: u32, bit_count: u16) -> usize {
    let bits = (width as usize) * (bit_count as usize);
    bits.div_ceil(32) * 4
}

fn header(width: u32, height: u32, bit_count: u16, image_bytes: usize) -> [u8; 40] {
    let mut h = [0u8; 40];
    h[0..4].copy_from_slice(&40u32.to_le_bytes()); // biSize
    h[4..8].copy_from_slice(&(width as i32).to_le_bytes()); // biWidth
                                                            // Negative height: top-down DIB, rows in display order.
    h[8..12].copy_from_slice(&(-(height as i32)).to_le_bytes()); // biHeight
    h[12..14].copy_from_slice(&1u16.to_le_bytes()); // biPlanes
    h[14..16].copy_from_slice(&bit_count.to_le_bytes()); // biBitCount
                                                         // biCompression = 0 (BI_RGB) at offset 16: already zero.
    h[20..24].copy_from_slice(&(image_bytes as u32).to_le_bytes()); // biSizeImage
    h
}

fn repack_rows(src: &[u8], src_row_bytes: usize, w: u32, h: u32, bit_count: u16) -> Vec<u8> {
    let dst_row = dib_row_bytes(w, bit_count);
    let mut out = vec![0u8; dst_row * h as usize];
    for row in 0..h as usize {
        out[row * dst_row..row * dst_row + src_row_bytes]
            .copy_from_slice(&src[row * src_row_bytes..(row + 1) * src_row_bytes]);
    }
    out
}

/// Build the DIB for one rendered page.
pub fn build_dib(page: &RenderedPage) -> DibImage {
    let (w, h) = (page.width_px, page.height_px);
    match page.mode {
        RenderMode::Color => {
            let bits = repack_rows(&page.pixels, w as usize * 4, w, h, 32);
            DibImage {
                width: w,
                height: h,
                header: header(w, h, 32, bits.len()),
                palette: Vec::new(),
                bits,
                bit_count: 32,
            }
        }
        RenderMode::Grayscale => {
            let mut palette = Vec::with_capacity(256 * 4);
            for i in 0..256u32 {
                palette.extend_from_slice(&[i as u8, i as u8, i as u8, 0]);
            }
            let bits = repack_rows(&page.pixels, w as usize, w, h, 8);
            DibImage {
                width: w,
                height: h,
                header: header(w, h, 8, bits.len()),
                palette,
                bits,
                bit_count: 8,
            }
        }
        RenderMode::Threshold => {
            // PDFium bits here are 1 = black; a DIB palette index 1
            // must be black, index 0 white, and PDFium/MonoBitmap bit
            // 1 means "dot set" — which maps directly onto palette
            // index 1. Rows are MSB-first in both layouts.
            let palette = vec![255, 255, 255, 0, 0, 0, 0, 0];
            let src_row = (w as usize).div_ceil(8);
            let bits = repack_rows(&page.pixels, src_row, w, h, 1);
            DibImage {
                width: w,
                height: h,
                header: header(w, h, 1, bits.len()),
                palette,
                bits,
                bit_count: 1,
            }
        }
    }
}

/// Fit an image into a printable area (device px), preserving aspect
/// ratio, centred. Returns (x, y, width, height) in device pixels.
/// Used for the StretchDIBits destination rectangle.
pub fn fit_to_page(img_w: u32, img_h: u32, area_w: u32, area_h: u32) -> (i32, i32, u32, u32) {
    if img_w == 0 || img_h == 0 || area_w == 0 || area_h == 0 {
        return (0, 0, 0, 0);
    }
    let scale = (area_w as f64 / img_w as f64).min(area_h as f64 / img_h as f64);
    let w = ((img_w as f64 * scale).floor() as u32).max(1).min(area_w);
    let h = ((img_h as f64 * scale).floor() as u32).max(1).min(area_h);
    let x = ((area_w - w) / 2) as i32;
    let y = ((area_h - h) / 2) as i32;
    (x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(mode: RenderMode, w: u32, h: u32, pixels: Vec<u8>) -> RenderedPage {
        RenderedPage {
            width_px: w,
            height_px: h,
            mode,
            dpi: 150,
            pixels,
        }
    }

    #[test]
    fn color_dib_header_and_stride() {
        // 3px wide: 12 data bytes/row -> already 4-aligned.
        let p = page(RenderMode::Color, 3, 2, vec![7u8; 3 * 2 * 4]);
        let d = build_dib(&p);
        assert_eq!(d.bit_count, 32);
        assert!(d.palette.is_empty());
        assert_eq!(d.bits.len(), 24);
        assert_eq!(u32::from_le_bytes(d.header[0..4].try_into().unwrap()), 40);
        assert_eq!(
            i32::from_le_bytes(d.header[8..12].try_into().unwrap()),
            -2,
            "top-down: negative height"
        );
    }

    #[test]
    fn grayscale_rows_pad_to_four_bytes() {
        // 5px wide gray: 5 bytes/row -> padded to 8.
        let p = page(RenderMode::Grayscale, 5, 3, vec![200u8; 15]);
        let d = build_dib(&p);
        assert_eq!(d.bit_count, 8);
        assert_eq!(d.palette.len(), 1024);
        assert_eq!(d.bits.len(), 8 * 3);
        assert_eq!(d.bits[0], 200);
        assert_eq!(d.bits[5], 0, "padding is zeroed");
        assert_eq!(dib_row_bytes(5, 8), 8);
    }

    #[test]
    fn threshold_palette_and_rows() {
        // 10px wide: 2 bytes/row -> padded to 4.
        let p = page(RenderMode::Threshold, 10, 2, vec![0xFF, 0xC0, 0x00, 0x00]);
        let d = build_dib(&p);
        assert_eq!(d.bit_count, 1);
        assert_eq!(&d.palette[0..4], &[255, 255, 255, 0], "index 0 = white");
        assert_eq!(&d.palette[4..8], &[0, 0, 0, 0], "index 1 = black");
        assert_eq!(d.bits.len(), 8);
        assert_eq!(d.bits[0], 0xFF);
        assert_eq!(dib_row_bytes(10, 1), 4);
    }

    #[test]
    fn fit_preserves_aspect_and_centres() {
        // A4 portrait image into a wider-than-tall printable area.
        let (x, y, w, h) = fit_to_page(1240, 1754, 2480, 1754);
        assert_eq!(h, 1754);
        assert!(w < 2480);
        assert_eq!(x, (2480 - w) as i32 / 2);
        assert_eq!(y, 0);
        // Exact-fit case passes through untouched.
        assert_eq!(fit_to_page(100, 100, 100, 100), (0, 0, 100, 100));
        // Degenerate input is safe.
        assert_eq!(fit_to_page(0, 10, 100, 100), (0, 0, 0, 0));
    }
}

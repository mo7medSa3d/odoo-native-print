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

fn checked_dib_row_bytes(width: u32, bit_count: u16) -> Option<usize> {
    let bits = (width as u64).checked_mul(bit_count as u64)?;
    usize::try_from(bits.div_ceil(32).checked_mul(4)?).ok()
}

/// Bytes per DIB row for a width and bit depth (4-byte aligned).
pub fn dib_row_bytes(width: u32, bit_count: u16) -> usize {
    checked_dib_row_bytes(width, bit_count).unwrap_or(usize::MAX)
}

fn header(width: u32, height: u32, bit_count: u16, image_bytes: usize) -> [u8; 40] {
    let mut h = [0u8; 40];
    let bi_width = i32::try_from(width).unwrap_or(0);
    // Negative height: top-down DIB, rows in display order.
    let bi_height = i32::try_from(height).map(|v| -v).unwrap_or(0);
    h[0..4].copy_from_slice(&40u32.to_le_bytes()); // biSize
    h[4..8].copy_from_slice(&bi_width.to_le_bytes()); // biWidth
    h[8..12].copy_from_slice(&bi_height.to_le_bytes()); // biHeight
    h[12..14].copy_from_slice(&1u16.to_le_bytes()); // biPlanes
    h[14..16].copy_from_slice(&bit_count.to_le_bytes()); // biBitCount
                                                         // biCompression = 0 (BI_RGB) at offset 16: already zero.
    h[20..24].copy_from_slice(&u32::try_from(image_bytes).unwrap_or(0).to_le_bytes()); // biSizeImage
    h
}

fn repack_rows(src: &[u8], src_row_bytes: usize, w: u32, h: u32, bit_count: u16) -> Vec<u8> {
    if w == 0 || h == 0 || i32::try_from(w).is_err() || i32::try_from(h).is_err() {
        return Vec::new();
    }
    let Some(dst_row) = checked_dib_row_bytes(w, bit_count).filter(|&r| r > 0) else {
        return Vec::new();
    };
    let Some(total) = dst_row.checked_mul(h as usize) else {
        return Vec::new();
    };
    if src_row_bytes == dst_row && src.len() >= total {
        return src[..total].to_vec();
    }
    // 8-bpp grayscale palette maps 0xFF to white paper (0x00 is solid black);
    // 1-bpp threshold maps 0 to white and 32-bpp BGRA keeps zeroed padding.
    let fill = if bit_count == 8 { 0xFF } else { 0x00 };
    let mut out = vec![fill; total];
    for (row, dst_slice) in out.chunks_exact_mut(dst_row).enumerate() {
        let Some(src_start) = row.checked_mul(src_row_bytes) else {
            break;
        };
        if src_start >= src.len() {
            break;
        }
        let copy_len = (src.len() - src_start).min(src_row_bytes).min(dst_row);
        dst_slice[..copy_len].copy_from_slice(&src[src_start..src_start + copy_len]);
    }
    out
}

const fn grayscale_palette() -> [u8; 256 * 4] {
    let mut pal = [0u8; 256 * 4];
    let mut i = 0;
    while i < 256 {
        let base = i * 4;
        pal[base] = i as u8;
        pal[base + 1] = i as u8;
        pal[base + 2] = i as u8;
        i += 1;
    }
    pal
}

const GRAYSCALE_PALETTE: [u8; 256 * 4] = grayscale_palette();

/// Build an 8-bpp grayscale DIB directly from a packed grayscale slice
/// without wrapping it in a `RenderedPage`.
pub fn build_grayscale_dib(width: u32, height: u32, pixels: &[u8]) -> DibImage {
    let bits = repack_rows(pixels, width as usize, width, height, 8);
    DibImage {
        width,
        height,
        header: header(width, height, 8, bits.len()),
        palette: GRAYSCALE_PALETTE.to_vec(),
        bits,
        bit_count: 8,
    }
}

/// Build the DIB for one rendered page.
pub fn build_dib(page: &RenderedPage) -> DibImage {
    let (w, h) = (page.width_px, page.height_px);
    match page.mode {
        RenderMode::Color => {
            let bits = (w as usize)
                .checked_mul(4)
                .map(|src_row| repack_rows(&page.pixels, src_row, w, h, 32))
                .unwrap_or_default();
            DibImage {
                width: w,
                height: h,
                header: header(w, h, 32, bits.len()),
                palette: Vec::new(),
                bits,
                bit_count: 32,
            }
        }
        RenderMode::Grayscale => build_grayscale_dib(w, h, &page.pixels),
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

/// Build the DIB taking ownership of `page.pixels` so stride-aligned pages
/// (32-bpp Color, 4px-aligned 8-bpp Grayscale, and 32px-aligned 1-bpp Threshold)
/// avoid cloning multi-megabyte buffers per page.
/// Fast single-pass check: true when every BGRA pixel is monochrome/grayscale
/// (B == G == R). Bails out on the first colored pixel in nanoseconds.
fn is_bgra_grayscale(pixels: &[u8]) -> bool {
    pixels
        .chunks_exact(4)
        .all(|px| px[0] == px[1] && px[1] == px[2])
}

pub fn build_dib_owned(mut page: RenderedPage) -> DibImage {
    let (w, h) = (page.width_px, page.height_px);
    if w > 0 && h > 0 && i32::try_from(w).is_ok() && i32::try_from(h).is_ok() {
        // Pure-B&W invoices/reports rendered in Color mode shrink 4x (34.8 MB -> 8.7 MB
        // per 300 DPI A4 page) when packed as an 8-bit grayscale DIB with zero visual loss.
        if page.mode == RenderMode::Color {
            if let Some(expected) = (w as usize)
                .checked_mul(h as usize)
                .and_then(|n| n.checked_mul(4))
            {
                if page.pixels.len() >= expected && is_bgra_grayscale(&page.pixels[..expected]) {
                    let gray_pixels: Vec<u8> = page.pixels[..expected]
                        .chunks_exact(4)
                        .map(|px| px[0])
                        .collect();
                    return build_dib_owned(RenderedPage {
                        width_px: w,
                        height_px: h,
                        mode: RenderMode::Grayscale,
                        dpi: page.dpi,
                        pixels: gray_pixels,
                    });
                }
            }
        }
        let (bit_count, src_row) = match page.mode {
            RenderMode::Color => (32u16, (w as usize).checked_mul(4)),
            RenderMode::Grayscale => (8u16, Some(w as usize)),
            RenderMode::Threshold => (1u16, Some((w as usize).div_ceil(8))),
        };
        if let (Some(src_row), Some(dst_row)) = (src_row, checked_dib_row_bytes(w, bit_count)) {
            if let Some(total) = dst_row.checked_mul(h as usize) {
                if src_row == dst_row && dst_row > 0 && page.pixels.len() >= total {
                    page.pixels.truncate(total);
                    let palette = match page.mode {
                        RenderMode::Color => Vec::new(),
                        RenderMode::Grayscale => GRAYSCALE_PALETTE.to_vec(),
                        RenderMode::Threshold => vec![255, 255, 255, 0, 0, 0, 0, 0],
                    };
                    return DibImage {
                        width: w,
                        height: h,
                        header: header(w, h, bit_count, total),
                        palette,
                        bits: page.pixels,
                        bit_count,
                    };
                }
            }
        }
    }
    build_dib(&page)
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
        assert_eq!(&d.palette[0..4], &[0, 0, 0, 0]);
        assert_eq!(&d.palette[128 * 4..128 * 4 + 4], &[128, 128, 128, 0]);
        assert_eq!(&d.palette[255 * 4..255 * 4 + 4], &[255, 255, 255, 0]);
        assert_eq!(d.bits.len(), 8 * 3);
        assert_eq!(d.bits[0], 200);
        assert_eq!(d.bits[5], 0xFF, "8-bpp padding is white paper");
        assert_eq!(dib_row_bytes(5, 8), 8);
        // Direct grayscale helper on a 4-byte-aligned width uses the fast path.
        let aligned = build_grayscale_dib(4, 2, &[10, 20, 30, 40, 50, 60, 70, 80]);
        assert_eq!(aligned.bit_count, 8);
        assert_eq!(aligned.bits, vec![10, 20, 30, 40, 50, 60, 70, 80]);
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

    #[test]
    fn signed_dimension_overflow_never_flips_top_down_or_allocates() {
        let d = build_grayscale_dib(1, (i32::MAX as u32) + 1, &[0u8; 4]);
        assert!(d.bits.is_empty());
        assert_eq!(i32::from_le_bytes(d.header[8..12].try_into().unwrap()), 0);
        let d2 = build_grayscale_dib((i32::MAX as u32) + 1, 1, &[0u8; 4]);
        assert!(d2.bits.is_empty());
        assert_eq!(i32::from_le_bytes(d2.header[4..8].try_into().unwrap()), 0);
    }

    #[test]
    fn owned_color_dib_moves_buffer_without_copy() {
        let mut pixels = vec![42u8; 4 * 3 * 4];
        // Include a real colored pixel (B != G != R) so auto-grayscale stays 32-bit.
        pixels[0] = 10;
        pixels[1] = 20;
        pixels[2] = 30;
        let ptr = pixels.as_ptr();
        let p = page(RenderMode::Color, 4, 3, pixels);
        let d = build_dib_owned(p);
        assert_eq!(d.bit_count, 32);
        assert_eq!(d.bits.len(), 48);
        assert_eq!(d.bits.as_ptr(), ptr);
    }

    #[test]
    fn owned_color_dib_auto_downgrades_pure_grayscale_pages_to_8bpp() {
        // 4x2 pure grayscale BGRA pixels -> packed as 8-bit DIB (4x smaller)
        let gray_bgra = vec![
            0x20, 0x20, 0x20, 0xFF, 0x80, 0x80, 0x80, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00,
            0x00, 0xFF, 0x40, 0x40, 0x40, 0xFF, 0x60, 0x60, 0x60, 0xFF, 0xA0, 0xA0, 0xA0, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF,
        ];
        let dib = build_dib_owned(page(RenderMode::Color, 4, 2, gray_bgra));
        assert_eq!(dib.bit_count, 8);
        assert_eq!(dib.bits.len(), 8);

        // One colored pixel keeps full 32-bit BGRA
        let mut colored_bgra = vec![0xFF; 4 * 2 * 4];
        colored_bgra[0] = 0x10; // B != G != R
        colored_bgra[1] = 0x80;
        colored_bgra[2] = 0xE0;
        let dib_color = build_dib_owned(page(RenderMode::Color, 4, 2, colored_bgra));
        assert_eq!(dib_color.bit_count, 32);
    }

    #[test]
    fn owned_color_dib_rejects_signed_dimension_overflow() {
        let p = page(RenderMode::Color, (i32::MAX as u32) + 1, 0, vec![0u8; 4]);
        let d = build_dib_owned(p);
        assert!(d.bits.is_empty());
        assert_eq!(i32::from_le_bytes(d.header[4..8].try_into().unwrap()), 0);
    }

    #[test]
    fn owned_grayscale_and_threshold_dib_move_aligned_buffers_without_copy() {
        let gray_pixels = vec![123u8; 8 * 3];
        let gray_ptr = gray_pixels.as_ptr();
        let gray = build_dib_owned(page(RenderMode::Grayscale, 8, 3, gray_pixels));
        assert_eq!(gray.bit_count, 8);
        assert_eq!(gray.palette.len(), 1024);
        assert_eq!(gray.bits.len(), 24);
        assert_eq!(gray.bits.as_ptr(), gray_ptr);

        let mono_pixels = vec![0xAAu8; 4 * 2];
        let mono_ptr = mono_pixels.as_ptr();
        let mono = build_dib_owned(page(RenderMode::Threshold, 32, 2, mono_pixels));
        assert_eq!(mono.bit_count, 1);
        assert_eq!(mono.palette.len(), 8);
        assert_eq!(mono.bits.len(), 8);
        assert_eq!(mono.bits.as_ptr(), mono_ptr);

        // Unaligned grayscale width still falls back to 4-byte padded repacking.
        let unaligned = build_dib_owned(page(RenderMode::Grayscale, 5, 2, vec![77u8; 10]));
        assert_eq!(unaligned.bits.len(), 16);
        assert_eq!(unaligned.bits[5], 0xFF);
    }

    #[test]
    fn grayscale_truncated_rows_pad_with_white_not_black() {
        let d = build_grayscale_dib(4, 3, &[10, 20, 30, 40, 50, 60]);
        assert_eq!(d.bit_count, 8);
        assert_eq!(
            d.bits,
            vec![10, 20, 30, 40, 50, 60, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
        );
    }
}

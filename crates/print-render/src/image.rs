//! Image jobs: PNG receipts and pictures.
//!
//! The POS extension sends the receipt as a PNG image (that is also how
//! Odoo's own IoT path prints receipts). This module is the native side
//! of that pipeline: decode the PNG defensively, convert to grayscale
//! (alpha composited over white), area-average downscale to the receipt
//! printer's printable width, threshold to a 1-bit bitmap, and encode
//! the ESC/POS raster stream. The same decoded image is what a GDI
//! backend prints for office printers (Windows phase).

use crate::bitmap::Canvas;
use print_raw::EscPos;
use std::io::Cursor;
use thiserror::Error;

pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 50_000_000;
pub const MAX_IMAGE_SIDE: u32 = 20_000;
/// Receipt threshold: receipt text is pure black on white; 160 keeps
/// anti-aliased edges while never inventing dots from paper-white.
pub const RECEIPT_THRESHOLD: u8 = 160;

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("image is empty or larger than the 20 MiB limit")]
    TooLarge,
    #[error("not a PNG image")]
    NotPng,
    #[error("image dimensions {0}x{1} exceed the safety limits")]
    DimensionsTooLarge(u32, u32),
    #[error("PNG decode failed: {0}")]
    Decode(String),
    #[error("PNG frame has an inconsistent size")]
    BadFrame,
}

/// Printable width in dots at the standard 203 dpi receipt density:
/// 80 mm paper -> 72 mm printable = 576 dots; 58 mm -> 48 mm = 384 dots.
pub fn receipt_dots_for_paper(paper: &str) -> u32 {
    match paper.trim() {
        "58mm" => 384,
        _ => 576, // 80mm is the default receipt width.
    }
}

fn luma_over_white(r: u8, g: u8, b: u8, a: u8) -> u8 {
    let a = a as u32;
    let blend = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
    let (r, g, b) = (blend(r), blend(g), blend(b));
    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
}

/// Decode a PNG into a grayscale Canvas, with hard limits checked from
/// the header BEFORE any frame buffer is allocated.
pub fn decode_png(bytes: &[u8]) -> Result<Canvas, ImageError> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(ImageError::TooLarge);
    }
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Err(ImageError::NotPng);
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| ImageError::Decode(e.to_string()))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0
        || height == 0
        || width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || width as u64 * height as u64 > MAX_IMAGE_PIXELS
    {
        return Err(ImageError::DimensionsTooLarge(width, height));
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| ImageError::Decode(e.to_string()))?;
    let data = &buf[..frame.buffer_size()];
    let px_count = (width * height) as usize;
    let mut px = vec![255u8; px_count];
    match frame.color_type {
        png::ColorType::Grayscale => {
            if data.len() < px_count {
                return Err(ImageError::BadFrame);
            }
            px.copy_from_slice(&data[..px_count]);
        }
        png::ColorType::GrayscaleAlpha => {
            if data.len() < px_count * 2 {
                return Err(ImageError::BadFrame);
            }
            for i in 0..px_count {
                px[i] = luma_over_white(data[2 * i], data[2 * i], data[2 * i], data[2 * i + 1]);
            }
        }
        png::ColorType::Rgb => {
            if data.len() < px_count * 3 {
                return Err(ImageError::BadFrame);
            }
            for i in 0..px_count {
                px[i] = luma_over_white(data[3 * i], data[3 * i + 1], data[3 * i + 2], 255);
            }
        }
        png::ColorType::Rgba => {
            if data.len() < px_count * 4 {
                return Err(ImageError::BadFrame);
            }
            for i in 0..px_count {
                px[i] = luma_over_white(
                    data[4 * i],
                    data[4 * i + 1],
                    data[4 * i + 2],
                    data[4 * i + 3],
                );
            }
        }
        png::ColorType::Indexed => {
            // EXPAND should have converted palettes already; refuse
            // rather than guess if a decoder build ever returns this.
            return Err(ImageError::Decode("indexed colour not expanded".into()));
        }
    }
    Ok(Canvas { width, height, px })
}

/// Area-average downscale to `target_width` (never upscale: a small
/// image prints at its natural size, centred by the printer backend).
/// Averaging (not nearest neighbour) is what keeps small receipt text
/// legible when a 2x browser capture is scaled to 576 dots.
pub fn scale_to_width(img: &Canvas, target_width: u32) -> Canvas {
    if target_width == 0 || img.width <= target_width {
        return Canvas {
            width: img.width,
            height: img.height,
            px: img.px.clone(),
        };
    }
    let ratio = img.width as f64 / target_width as f64;
    let out_height = ((img.height as f64 / ratio).round() as u32).max(1);
    let mut px = vec![255u8; (target_width * out_height) as usize];
    for oy in 0..out_height {
        let y0 = (oy as f64 * ratio) as u32;
        let y1 = (((oy + 1) as f64 * ratio).ceil() as u32)
            .min(img.height)
            .max(y0 + 1);
        for ox in 0..target_width {
            let x0 = (ox as f64 * ratio) as u32;
            let x1 = (((ox + 1) as f64 * ratio).ceil() as u32)
                .min(img.width)
                .max(x0 + 1);
            let mut sum: u64 = 0;
            let mut count: u64 = 0;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += img.px[(y * img.width + x) as usize] as u64;
                    count += 1;
                }
            }
            px[(oy * target_width + ox) as usize] = (sum / count.max(1)) as u8;
        }
    }
    Canvas {
        width: target_width,
        height: out_height,
        px,
    }
}

/// The full receipt pipeline: PNG bytes -> ready-to-send ESC/POS stream
/// (raster + feed + partial cut), sized for the profile's paper.
pub fn receipt_png_to_escpos(png_bytes: &[u8], paper: &str) -> Result<Vec<u8>, ImageError> {
    let img = decode_png(png_bytes)?;
    let scaled = scale_to_width(&img, receipt_dots_for_paper(paper));
    let mono = scaled.to_mono(RECEIPT_THRESHOLD);
    let stream = EscPos::new().raster(&mono).feed_lines(3).cut_partial();
    Ok(stream.bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_png(width: u32, height: u32, color: png::ColorType, pixels: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixels).unwrap();
        }
        out
    }

    fn half_black_rgba_png(width: u32, height: u32) -> Vec<u8> {
        let mut px = Vec::new();
        for _y in 0..height {
            for x in 0..width {
                if x < width / 2 {
                    px.extend_from_slice(&[0, 0, 0, 255]);
                } else {
                    px.extend_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        make_png(width, height, png::ColorType::Rgba, &px)
    }

    #[test]
    fn decode_rgba_halves() {
        let bytes = half_black_rgba_png(4, 2);
        let img = decode_png(&bytes).unwrap();
        assert_eq!((img.width, img.height), (4, 2));
        assert_eq!(img.px[0], 0);
        assert_eq!(img.px[3], 255);
    }
    #[test]
    fn transparent_pixels_become_white() {
        // A fully transparent black pixel prints as paper, never as ink.
        let bytes = make_png(1, 1, png::ColorType::Rgba, &[0, 0, 0, 0]);
        let img = decode_png(&bytes).unwrap();
        assert_eq!(img.px[0], 255);
    }
    #[test]
    fn garbage_and_oversize_rejected() {
        assert!(matches!(decode_png(b"not a png"), Err(ImageError::NotPng)));
        assert!(matches!(decode_png(&[]), Err(ImageError::TooLarge)));
        // 20,001 px wide exceeds the per-side cap before allocation.
        let wide = make_png(20_001, 1, png::ColorType::Grayscale, &vec![255u8; 20_001]);
        assert!(matches!(
            decode_png(&wide),
            Err(ImageError::DimensionsTooLarge(20_001, 1))
        ));
    }
    #[test]
    fn scale_averages_blocks() {
        let mut img = Canvas::new(8, 1);
        img.px = vec![0, 0, 255, 255, 0, 0, 255, 255];
        let out = scale_to_width(&img, 4);
        assert_eq!(out.px, vec![0, 255, 0, 255]);
        // Never upscale.
        let same = scale_to_width(&img, 576);
        assert_eq!(same.width, 8);
    }
    #[test]
    fn receipt_stream_shape_80mm_and_58mm() {
        // A 1152px-wide capture (2x of 576) scales to exactly 576 dots.
        let bytes = half_black_rgba_png(1152, 20);
        let stream = receipt_png_to_escpos(&bytes, "80mm").unwrap();
        assert!(stream.starts_with(&[0x1B, 0x40])); // ESC @
        let pos = stream
            .windows(4)
            .position(|w| w == [0x1D, 0x76, 0x30, 0x00])
            .unwrap();
        let bytes_per_row = u16::from_le_bytes([stream[pos + 4], stream[pos + 5]]);
        assert_eq!(bytes_per_row, 576 / 8);
        assert!(stream.ends_with(&[0x1D, 0x56, 0x01])); // partial cut
                                                        // 58mm: 384 dots wide.
        let stream58 = receipt_png_to_escpos(&bytes, "58mm").unwrap();
        let pos58 = stream58
            .windows(4)
            .position(|w| w == [0x1D, 0x76, 0x30, 0x00])
            .unwrap();
        let bpr58 = u16::from_le_bytes([stream58[pos58 + 4], stream58[pos58 + 5]]);
        assert_eq!(bpr58, 384 / 8);
    }
    #[test]
    fn paper_widths() {
        assert_eq!(receipt_dots_for_paper("80mm"), 576);
        assert_eq!(receipt_dots_for_paper("58mm"), 384);
        assert_eq!(receipt_dots_for_paper(""), 576);
    }
}

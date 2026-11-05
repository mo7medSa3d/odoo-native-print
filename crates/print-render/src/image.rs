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
/// - `58mm` (or `57mm`/`58 mm` roll) -> 48 mm printable = 384 dots
/// - `80mm` (or `72mm`/`80 mm` roll) -> 72 mm printable = 576 dots
pub fn receipt_dots_for_paper(paper: &str) -> Option<u32> {
    let lower = paper.trim().to_ascii_lowercase();
    if lower.starts_with("58")
        || lower.contains("58mm")
        || lower.contains("58 mm")
        || lower.starts_with("57")
    {
        Some(384)
    } else if lower.starts_with("112") || lower.contains("112mm") || lower.contains("112 mm") {
        Some(832)
    } else if lower.starts_with("80")
        || lower.contains("80mm")
        || lower.contains("80 mm")
        || lower.starts_with("72")
    {
        Some(576)
    } else {
        // Unknown paper names are refused, never silently treated as
        // 80 mm (a wrong-width receipt is clipped or scaled).
        None
    }
}

/// Map a receipt paper width (mm) to the ESC/POS raster width in
/// dots at 203 dpi. Widths with verified head profiles keep their
/// exact dot counts (58 mm paper -> 48 mm printable = 384; 80/72 mm
/// -> 72 mm = 576; 112 mm -> 104 mm = 832). Any other plausible
/// receipt width uses the same physical model those profiles
/// encode — the printable area is the paper minus ~8 mm of head
/// margins, at ~8 dots per mm, rounded down to whole raster bytes
/// (GS v 0 rows are byte-addressed). A POS printer prints at ITS
/// width; it is never refused for missing a catalogue entry
/// (owner directive, 2026-11-05: POS prints to any printer, sized
/// to that printer's paper). Widths outside the receipt range are
/// not receipt media and return None.
pub fn receipt_dots_for_width_mm(width_mm: f64) -> Option<u32> {
    if !width_mm.is_finite() || width_mm <= 0.0 {
        return None;
    }
    if (width_mm - 58.0).abs() <= 4.0 {
        Some(384)
    } else if (width_mm - 80.0).abs() <= 8.0 || (width_mm - 72.0).abs() <= 4.0 {
        Some(576)
    } else if (width_mm - 112.0).abs() <= 8.0 {
        Some(832)
    } else if (40.0..=220.0).contains(&width_mm) {
        let printable_mm = width_mm - 8.0;
        let dots = (printable_mm * 203.0 / 25.4 / 8.0).floor() as u32 * 8;
        Some(dots.clamp(256, 1664))
    } else {
        None
    }
}

/// Render a receipt image using the print-head width resolved from the driver.
pub fn receipt_png_to_escpos_width(
    png_bytes: &[u8],
    target_width_dots: u32,
) -> Result<Vec<u8>, ImageError> {
    if !(256..=2048).contains(&target_width_dots) || target_width_dots % 8 != 0 {
        return Err(ImageError::DimensionsTooLarge(target_width_dots, 1));
    }
    let img = decode_png(png_bytes)?;
    let scaled = scale_to_width(&img, target_width_dots);
    let mono = scaled.to_mono(RECEIPT_THRESHOLD);
    let stream = EscPos::new().raster(&mono).feed_lines(3).cut_partial();
    Ok(stream.bytes().to_vec())
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
    let dots = receipt_dots_for_paper(paper)
        .ok_or_else(|| ImageError::Decode(format!("unknown receipt paper {paper:?}")))?;
    let scaled = scale_to_width(&img, dots);
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
    fn driver_roll_width_maps_only_known_receipt_heads() {
        assert_eq!(receipt_dots_for_width_mm(58.0), Some(384));
        assert_eq!(receipt_dots_for_width_mm(80.0), Some(576));
        assert_eq!(receipt_dots_for_width_mm(112.0), Some(832));
        // Non-catalogue widths follow the printable-width model
        // (paper - 8 mm at 203 dpi, whole bytes): POS prints at
        // whatever width the printer actually has.
        assert_eq!(receipt_dots_for_width_mm(100.0), Some(728));
        assert_eq!(receipt_dots_for_width_mm(90.0), Some(648));
        assert_eq!(receipt_dots_for_width_mm(76.0), Some(576));
        assert_eq!(receipt_dots_for_width_mm(44.0), Some(280));
        assert_eq!(receipt_dots_for_width_mm(210.0), Some(1608));
        // Outside the receipt range there is no honest mapping.
        assert_eq!(receipt_dots_for_width_mm(30.0), None);
        assert_eq!(receipt_dots_for_width_mm(250.0), None);
        assert_eq!(receipt_dots_for_width_mm(f64::NAN), None);
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
        let bytes = make_png(1, 1, png::ColorType::Rgba, &[0, 0, 0, 0]);
        let img = decode_png(&bytes).unwrap();
        assert_eq!(img.px[0], 255);
    }
    #[test]
    fn garbage_and_oversize_rejected() {
        assert!(matches!(decode_png(b"not a png"), Err(ImageError::NotPng)));
        assert!(matches!(decode_png(&[]), Err(ImageError::TooLarge)));
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
        let same = scale_to_width(&img, 576);
        assert_eq!(same.width, 8);
    }
    #[test]
    fn receipt_stream_shape_80mm_and_58mm() {
        let bytes = half_black_rgba_png(1152, 20);
        let stream = receipt_png_to_escpos(&bytes, "80mm").unwrap();
        assert!(stream.starts_with(&[0x1B, 0x40]));
        let pos = stream
            .windows(4)
            .position(|w| w == [0x1D, 0x76, 0x30, 0x00])
            .unwrap();
        let bytes_per_row = u16::from_le_bytes([stream[pos + 4], stream[pos + 5]]);
        assert_eq!(bytes_per_row, 576 / 8);
        assert!(stream.ends_with(&[0x1D, 0x56, 0x01]));
        let stream58 = receipt_png_to_escpos(&bytes, "58mm").unwrap();
        let pos58 = stream58
            .windows(4)
            .position(|w| w == [0x1D, 0x76, 0x30, 0x00])
            .unwrap();
        let bpr58 = u16::from_le_bytes([stream58[pos58 + 4], stream58[pos58 + 5]]);
        assert_eq!(bpr58, 384 / 8);
    }
    /// Parse every GS v 0 raster header in a stream: (bytes_per_row,
    /// rows) per band, in order.
    fn raster_bands(stream: &[u8]) -> Vec<(u16, u16)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 8 <= stream.len() {
            if stream[i..].starts_with(&[0x1D, 0x76, 0x30, 0x00]) {
                let x = u16::from_le_bytes([stream[i + 4], stream[i + 5]]);
                let y = u16::from_le_bytes([stream[i + 6], stream[i + 7]]);
                out.push((x, y));
                i += 8 + (x as usize) * (y as usize);
            } else {
                i += 1;
            }
        }
        out
    }

    #[test]
    fn tall_receipt_is_banded_and_complete() {
        // A long receipt (576 x 2000 dots at scale 1) must arrive as
        // several raster bands whose heights sum to the full image:
        // nothing truncated, nothing duplicated, then feed + cut.
        let bytes = half_black_rgba_png(576, 2000);
        let stream = receipt_png_to_escpos_width(&bytes, 576).unwrap();
        let bands = raster_bands(&stream);
        assert!(bands.len() > 1, "tall receipt must be banded: {bands:?}");
        assert!(bands.iter().all(|&(x, _)| x == 576 / 8));
        let total: u32 = bands.iter().map(|&(_, y)| u32::from(y)).sum();
        assert_eq!(total, 2000);
        assert!(stream.ends_with(&[0x1B, 0x64, 3, 0x1D, 0x56, 0x01]));
    }

    #[test]
    fn wide_receipt_downscales_proportionally_to_roll() {
        // 1152x24 source onto a 58mm (384-dot) head: width 384 and
        // height scaled by the same ratio (24 * 384/1152 = 8).
        let bytes = half_black_rgba_png(1152, 24);
        let stream = receipt_png_to_escpos_width(&bytes, 384).unwrap();
        let bands = raster_bands(&stream);
        assert_eq!(bands, vec![(384 / 8, 8)]);
    }

    #[test]
    fn receipt_112mm_head_and_bad_targets() {
        let bytes = half_black_rgba_png(832, 8);
        let stream = receipt_png_to_escpos_width(&bytes, 832).unwrap();
        assert_eq!(raster_bands(&stream), vec![(832 / 8, 8)]);
        // Not a multiple of 8, below the floor, above the ceiling
        // (the ceiling rose to 2048 with generalized widths: a
        // 210 mm receipt head legitimately renders 1608 dots).
        for bad in [100u32, 128, 2056] {
            assert!(
                receipt_png_to_escpos_width(&bytes, bad).is_err(),
                "target {bad} must be rejected"
            );
        }
    }

    #[test]
    fn paper_widths() {
        assert_eq!(receipt_dots_for_paper("80mm"), Some(576));
        assert_eq!(receipt_dots_for_paper("80mm Roll"), Some(576));
        assert_eq!(receipt_dots_for_paper("58mm"), Some(384));
        assert_eq!(receipt_dots_for_paper("58 mm Roll"), Some(384));
        assert_eq!(receipt_dots_for_paper("58x3276mm"), Some(384));
        assert_eq!(receipt_dots_for_paper(""), None);
        assert_eq!(receipt_dots_for_paper("112mm"), Some(832));
    }
}

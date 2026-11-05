//! The built-in calibration/test page (spec section 8): a field engineer
//! prints it and validates any printer in about two minutes — measure the
//! 100 mm square, check the rulers reach the edges (clipping), scan the
//! barcode, and follow the hardware kit checklist.
//!
//! Arabic validation note: an A4 calibration PDF cannot embed an Arabic
//! font honestly from this generator, so Arabic is validated where it
//! actually prints: the Odoo receipt/report path (wkhtmltopdf-embedded
//! fonts on office printers, and the raster ESC/POS path for thermal).
//! The thermal calibration bitmap below is that raster path's test page.

use crate::bitmap::Canvas;
use print_raw::MonoBitmap;

/// Code 39 wide/narrow patterns, 9 elements per character, elements
/// alternate bar/space starting with a bar; '1' = wide. Table from the
/// Code 39 specification. The start/stop character is '*'.
fn code39_pattern(ch: char) -> Option<[bool; 9]> {
    let bits = match ch {
        '0' => "000110100",
        '1' => "100100001",
        '2' => "001100001",
        '3' => "101100000",
        '4' => "000110001",
        '5' => "100110000",
        '6' => "001110000",
        '7' => "000100101",
        '8' => "100100100",
        '9' => "001100100",
        'A' => "100001001",
        'B' => "001001001",
        'C' => "101001000",
        'D' => "000011001",
        'E' => "100011000",
        'F' => "001011000",
        'G' => "000001101",
        'H' => "100001100",
        'I' => "001001100",
        'J' => "000011100",
        'K' => "100000011",
        'L' => "001000011",
        'M' => "101000010",
        'N' => "000010011",
        'O' => "100010010",
        'P' => "001010010",
        'Q' => "000000111",
        'R' => "100000110",
        'S' => "001000110",
        'T' => "000010110",
        'U' => "110000001",
        'V' => "011000001",
        'W' => "111000000",
        'X' => "010010001",
        'Y' => "110010000",
        'Z' => "011010000",
        '-' => "010000101",
        '.' => "110000100",
        ' ' => "011000100",
        '$' => "010101000",
        '/' => "010100010",
        '+' => "010001010",
        '%' => "000101010",
        '*' => "010010100",
        _ => return None,
    };
    let mut out = [false; 9];
    for (i, b) in bits.bytes().enumerate() {
        out[i] = b == b'1';
    }
    Some(out)
}

fn code39_value(ch: char) -> Option<u32> {
    "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%"
        .chars()
        .position(|c| c == ch)
        .map(|p| p as u32)
}

/// Bar widths (in units) for a Code 39 string, with start/stop and a
/// modulo-43 check character. Narrow = 1 unit, wide = 3 units; one narrow
/// space separates characters. Returns (is_bar, width) runs including
/// inter-character gaps as spaces. Unsupported characters produce an error string.
pub fn code39_runs(data: &str) -> Result<Vec<(bool, u32)>, String> {
    let data = data.to_uppercase();
    let mut sum: u32 = 0;
    for ch in data.chars() {
        sum += code39_value(ch).ok_or_else(|| format!("unsupported character: {ch}"))?;
    }
    let check = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%"
        .chars()
        .nth((sum % 43) as usize)
        .unwrap_or('0');
    let full = format!("*{data}{check}*");
    let mut runs: Vec<(bool, u32)> = Vec::new();
    for (ci, ch) in full.chars().enumerate() {
        let pattern = code39_pattern(ch).ok_or_else(|| format!("no pattern for {ch}"))?;
        for (ei, wide) in pattern.iter().enumerate() {
            let is_bar = ei % 2 == 0;
            runs.push((is_bar, if *wide { 3 } else { 1 }));
        }
        if ci + 1 < full.len() {
            runs.push((false, 1)); // inter-character gap
        }
    }
    Ok(runs)
}

fn pdf_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// A one-page A4 calibration PDF (595.28 x 841.89 pt).
pub fn calibration_pdf() -> Vec<u8> {
    calibration_pdf_sized(210.0, 297.0)
}

/// A one-page calibration PDF at an arbitrary page size, generated
/// with no external dependencies: page border, rulers on the
/// top/left edges with ticks every 10 mm, an exact sizing square
/// (100x100 mm when the page fits it, otherwise the largest that
/// fits, with its size printed on the page), a Code 39 barcode for
/// TEST-2026 scaled to the page width, the page size itself
/// printed as text, and instructions. This is what lets the
/// calibration lab test the size the user actually chose instead
/// of always emitting A4 (owner directive, 2026-11-05).
pub fn calibration_pdf_sized(width_mm: f64, height_mm: f64) -> Vec<u8> {
    let w = width_mm.clamp(30.0, 1_500.0);
    let h = height_mm.clamp(30.0, 1_500.0);
    let mm = 72.0f64 / 25.4;
    let margin = if w.min(h) >= 60.0 { 10.0 } else { 5.0 };
    let mut c = String::new();
    c.push_str("2 J\n"); // line joins
                         // Page border inset by the margin.
    c.push_str(&format!(
        "{:.2} {:.2} {:.2} {:.2} re S\n",
        margin * mm,
        margin * mm,
        (w - 2.0 * margin) * mm,
        (h - 2.0 * margin) * mm
    ));
    // Rulers: top edge ticks every 10 mm across the usable width,
    // left edge ticks every 10 mm across the usable height.
    let tick_top_y = h - margin;
    let mut tx = margin;
    let mut i = 0;
    while tx <= w - margin + 0.01 {
        let len = if i % 5 == 0 { 8.0 } else { 4.0 } * mm;
        c.push_str(&format!(
            "{:.2} {:.2} m {:.2} {:.2} l S\n",
            tx * mm,
            tick_top_y * mm,
            tx * mm,
            tick_top_y * mm - len
        ));
        tx += 10.0;
        i += 1;
    }
    let mut ty = margin;
    let mut j = 0;
    while ty <= h - margin + 0.01 {
        let len = if j % 5 == 0 { 8.0 } else { 4.0 } * mm;
        c.push_str(&format!(
            "{:.2} {:.2} m {:.2} {:.2} l S\n",
            margin * mm,
            ty * mm,
            margin * mm + len,
            ty * mm
        ));
        ty += 10.0;
        j += 1;
    }
    // Sizing square: 100 mm when it fits with room for the texts,
    // otherwise the largest square the usable area holds.
    let side = 100.0f64
        .min((w - 2.0 * margin) * 0.55)
        .min((h - 2.0 * margin) * 0.42)
        .max(15.0);
    let sq_x = (w - side) / 2.0;
    let sq_y = h * 0.52;
    c.push_str(&format!(
        "{:.2} {:.2} {:.2} {:.2} re S\n",
        sq_x * mm,
        sq_y * mm,
        side * mm,
        side * mm
    ));
    // Text (Helvetica, a PDF base-14 font: no embedding needed).
    let text = |x_mm: f64, y_mm: f64, size: u32, s: &str| -> String {
        format!(
            "BT /F1 {size} Tf {:.2} {:.2} Td ({}) Tj ET\n",
            x_mm * mm,
            y_mm * mm,
            pdf_escape(s)
        )
    };
    let title_size = if w < 120.0 { 12 } else { 16 };
    let body_size = if w < 120.0 { 8 } else { 11 };
    let tx0 = margin + 4.0;
    c.push_str(&text(
        tx0,
        h - margin - 14.0,
        title_size,
        "Yaseir Print - Calibration Page",
    ));
    c.push_str(&text(
        tx0,
        h - margin - 22.0,
        body_size,
        &format!("Page size: {:.0} x {:.0} mm", w, h),
    ));
    c.push_str(&text(
        tx0,
        sq_y - 8.0,
        body_size,
        &format!(
            "The square above must measure exactly {:.0} x {:.0} mm.",
            side, side
        ),
    ));
    c.push_str(&text(
        tx0,
        sq_y - 14.0,
        body_size,
        "Ruler ticks are 10 mm apart. Check nothing is clipped at the edges.",
    ));
    // Barcode: Code 39 for TEST-2026, unit scaled so the code fits
    // the usable width, height 15 mm (or a quarter of the page).
    if let Ok(runs) = code39_runs("TEST-2026") {
        let total_units: u32 = runs.iter().map(|(_, n)| *n).sum();
        if total_units > 0 {
            let unit_mm = 0.9f64.min((w - 2.0 * margin - 8.0) / total_units as f64);
            if unit_mm > 0.2 {
                let unit = unit_mm * mm;
                let bar_h = 15.0f64.min(h * 0.22);
                let total_w = total_units as f64 * unit;
                let mut x = (w * mm - total_w) / 2.0;
                let y0 = (margin + 12.0) * mm;
                for (is_bar, n) in runs {
                    let width = n as f64 * unit;
                    if is_bar {
                        c.push_str(&format!(
                            "{x:.2} {y0:.2} {width:.2} {:.2} re f\n",
                            bar_h * mm
                        ));
                    }
                    x += width;
                }
                c.push_str(&text(
                    tx0,
                    margin + 6.0,
                    body_size,
                    "Scan the barcode: it must read TEST-2026 plus one check character.",
                ));
            }
        }
    }

    let content = c.into_bytes();
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
            w * mm,
            h * mm
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    offsets.push(out.len());
    out.extend_from_slice(b"5 0 obj\n");
    out.extend_from_slice(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
    out.extend_from_slice(&content);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let xref_pos = out.len();
    out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer << /Size 6 /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF").as_bytes(),
    );
    out
}

/// Thermal calibration bitmap for a thermal receipt printer at 203 dpi
/// (`width_dots`: 576 dots for 80 mm paper, 384 dots for 58 mm paper).
pub fn calibration_thermal_for_width(width_dots: u32) -> MonoBitmap {
    let width = width_dots.max(384);
    let dpi = 203.0f64;
    let mm = |v: f64| (v * dpi / 25.4) as u32;
    let height = mm(80.0);
    let mut canvas = Canvas::new(width, height);
    canvas.rect_outline(4, 4, width - 8, height - 8, 2);
    // 30x30mm square centred horizontally at the top.
    let sq = mm(30.0);
    canvas.rect_outline((width - sq) / 2, mm(8.0), sq, sq, 2);
    // Code 39 bars under the square.
    if let Ok(runs) = code39_runs("TEST-2026") {
        let raw_units: u32 = runs.iter().map(|(_, w)| *w).sum();
        let unit: u32 = if raw_units * 3 + 16 <= width { 3 } else { 2 };
        let total: u32 = raw_units * unit;
        let mut x = width.saturating_sub(total) / 2;
        let y = mm(44.0);
        for (is_bar, w) in runs {
            if is_bar {
                for dx in 0..(w * unit) {
                    canvas.vline(x + dx, y, y + mm(12.0), 1);
                }
            }
            x += w * unit;
        }
    }
    canvas.to_mono(128)
}

/// Thermal calibration bitmap for an 80 mm printer at 203 dpi (576 dots).
pub fn calibration_thermal_80mm() -> MonoBitmap {
    calibration_thermal_for_width(576)
}

/// Thermal calibration bitmap for a 58 mm printer at 203 dpi (384 dots).
pub fn calibration_thermal_58mm() -> MonoBitmap {
    calibration_thermal_for_width(384)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code39_check_char_and_roundtrip() {
        let runs = code39_runs("A").unwrap();
        let mut chars = Vec::new();
        let mut i = 0;
        while i < runs.len() {
            if i + 9 > runs.len() {
                break;
            }
            let mut pattern = [false; 9];
            for (j, item) in pattern.iter_mut().enumerate() {
                *item = runs[i + j].1 == 3;
            }
            let ch = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%*"
                .chars()
                .find(|&c| code39_pattern(c) == Some(pattern));
            chars.push(ch.unwrap_or('?'));
            i += 10;
        }
        assert_eq!(chars, vec!['*', 'A', 'A', '*']);
    }
    #[test]
    fn code39_rejects_bad_chars() {
        assert!(code39_runs("hello@").is_err());
    }
    #[test]
    fn calibration_pdf_is_a_pdf() {
        let pdf = calibration_pdf();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("/MediaBox [0 0 595.28 841.89]"));
        assert!(s.contains("Calibration Page"));
    }
    #[test]
    fn calibration_pdf_sized_carries_the_requested_page() {
        // A5: the MediaBox must be the A5 size in points and the
        // printed page-size line must name it, so a lab test at a
        // chosen size is verifiable on the paper itself.
        let pdf = calibration_pdf_sized(148.0, 210.0);
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("/MediaBox [0 0 419.53 595.28]"), "{s:.200}");
        assert!(s.contains("Page size: 148 x 210 mm"));
        // A label size still produces a valid, parseable one-pager.
        let info = crate::pdf::parse_pdf_info(&calibration_pdf_sized(100.0, 150.0)).unwrap();
        assert_eq!(info.pages.len(), 1);
        assert!((info.pages[0].size.width_mm - 100.0).abs() < 1.0);
        assert!((info.pages[0].size.height_mm - 150.0).abs() < 1.0);
    }
    #[test]
    fn thermal_calibration_bitmap_80_and_58() {
        let bm80 = calibration_thermal_80mm();
        assert_eq!(bm80.width_dots, 576);
        assert!(bm80.pixel_is_black(4, 4));
        assert!(!bm80.pixel_is_black(288, 200));

        let bm58 = calibration_thermal_58mm();
        assert_eq!(bm58.width_dots, 384);
        assert!(bm58.pixel_is_black(4, 4));
    }
}

use crate::mono::MonoBitmap;
use std::io::Write as _;
use thiserror::Error;

/// Maximum label width or height in dots accepted by CPCL encoders.
/// Caps header values to u16 range and avoids oversized allocations.
pub const MAX_DIMENSION_DOTS: u32 = u16::MAX as u32;

/// Maximum raw 1-bit graphic payload accepted by `bitmap_label` (16 MiB).
pub const MAX_BITMAP_BYTES: usize = 16 * 1024 * 1024;

/// Default DPI used by `text_label` (203 dpi mobile/desktop CPCL standard).
const DEFAULT_TEXT_DPI: u16 = 203;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CpclError {
    #[error("invalid DPI {0}: must be greater than zero")]
    InvalidDpi(u16),
    #[error("bitmap dimensions {width}x{height} must be non-zero")]
    EmptyDimensions { width: u32, height: u32 },
    #[error("bitmap dimensions {width}x{height} exceed CPCL limit ({MAX_DIMENSION_DOTS} dots)")]
    DimensionsTooLarge { width: u32, height: u32 },
    #[error("bitmap payload size {0} exceeds limit ({MAX_BITMAP_BYTES} bytes)")]
    PayloadTooLarge(usize),
    #[error("bitmap data length {got} is smaller than expected {expected} bytes")]
    TruncatedBitmap { expected: usize, got: usize },
}

/// Encode a 1-bit monochrome bitmap as a CPCL `CG` (Compressed Graphics) label.
///
/// CPCL `CG` uses bit `1` = black dot and bit `0` = white, matching `MonoBitmap`
/// directly without bitwise inversion.
pub fn bitmap_label(bitmap: &MonoBitmap, dpi: u16, copies: u16) -> Result<Vec<u8>, CpclError> {
    if dpi == 0 {
        return Err(CpclError::InvalidDpi(dpi));
    }
    if bitmap.width_dots == 0 || bitmap.height_dots == 0 {
        return Err(CpclError::EmptyDimensions {
            width: bitmap.width_dots,
            height: bitmap.height_dots,
        });
    }
    if bitmap.width_dots > MAX_DIMENSION_DOTS || bitmap.height_dots > MAX_DIMENSION_DOTS {
        return Err(CpclError::DimensionsTooLarge {
            width: bitmap.width_dots,
            height: bitmap.height_dots,
        });
    }

    let row_bytes = bitmap.bytes_per_row();
    let expected = row_bytes
        .checked_mul(bitmap.height_dots as usize)
        .ok_or(CpclError::PayloadTooLarge(usize::MAX))?;
    if expected == 0 {
        return Err(CpclError::EmptyDimensions {
            width: bitmap.width_dots,
            height: bitmap.height_dots,
        });
    }
    if expected > MAX_BITMAP_BYTES {
        return Err(CpclError::PayloadTooLarge(expected));
    }
    if bitmap.data.len() < expected {
        return Err(CpclError::TruncatedBitmap {
            expected,
            got: bitmap.data.len(),
        });
    }

    let copies = copies.max(1);
    const TRAILER: &[u8] = b"\r\nFORM\r\nPRINT\r\n";
    // Header (<= 57 B for u16-bounded dims) + trailer (15 B) <= 72 B; 80 B guarantees 0 reallocs.
    let mut out = Vec::with_capacity(expected.saturating_add(80));
    let _ = write!(
        out,
        "! 0 {dpi} {dpi} {} {copies}\r\nPW {}\r\nCG {row_bytes} {} 0 0 ",
        bitmap.height_dots, bitmap.width_dots, bitmap.height_dots
    );
    out.extend_from_slice(&bitmap.data[..expected]);
    out.extend_from_slice(TRAILER);
    Ok(out)
}

fn sanitize_cpcl_text(line: &str) -> String {
    // Strip all control characters (including CR, LF, ESC, NUL) so user data
    // cannot break out of a TEXT line into arbitrary CPCL commands.
    line.chars().filter(|c| !c.is_control()).collect()
}

/// Encode plain text lines as a CPCL label, sanitizing CR/LF and all control
/// bytes to prevent CPCL command injection.
pub fn text_label(lines: &[&str], width_dots: u16, height_dots: u16, copies: u16) -> Vec<u8> {
    let width = width_dots.max(1);
    let height = height_dots.max(1);
    let copies = copies.max(1);

    let mut out = Vec::with_capacity(64usize.saturating_add(lines.len().saturating_mul(32)));
    let _ = write!(
        out,
        "! 0 {DEFAULT_TEXT_DPI} {DEFAULT_TEXT_DPI} {height} {copies}\r\nPW {width}\r\n"
    );

    for (idx, line) in lines.iter().enumerate() {
        let safe = sanitize_cpcl_text(line);
        let y = 16u32.saturating_add((idx as u32).saturating_mul(28));
        let _ = write!(out, "TEXT 0 2 16 {y} {safe}\r\n");
    }

    out.extend_from_slice(b"FORM\r\nPRINT\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_label_header_and_payload_length() -> Result<(), Box<dyn std::error::Error>> {
        let raw = vec![0xFF, 0x00, 0xAA, 0x55];
        let bm = MonoBitmap::from_packed(16, 2, raw.clone())?;
        let out = bitmap_label(&bm, 203, 2)?;

        let expected_prefix = b"! 0 203 203 2 2\r\nPW 16\r\nCG 2 2 0 0 ";
        let expected_suffix = b"\r\nFORM\r\nPRINT\r\n";

        assert!(out.starts_with(expected_prefix));
        assert!(out.ends_with(expected_suffix));
        assert_eq!(
            out.len(),
            expected_prefix.len() + raw.len() + expected_suffix.len()
        );
        let payload = &out[expected_prefix.len()..out.len() - expected_suffix.len()];
        assert_eq!(payload, raw.as_slice());
        Ok(())
    }

    #[test]
    fn bitmap_label_clamps_zero_copies_to_one() -> Result<(), Box<dyn std::error::Error>> {
        let bm = MonoBitmap::from_packed(8, 1, vec![0x81])?;
        let out = bitmap_label(&bm, 203, 0)?;
        assert!(out.starts_with(b"! 0 203 203 1 1\r\nPW 8\r\nCG 1 1 0 0 "));
        Ok(())
    }

    #[test]
    fn bitmap_label_rejects_zero_and_oversized_dimensions() {
        let zero_w = MonoBitmap {
            width_dots: 0,
            height_dots: 10,
            data: vec![],
        };
        assert_eq!(
            bitmap_label(&zero_w, 203, 1),
            Err(CpclError::EmptyDimensions {
                width: 0,
                height: 10
            })
        );

        let zero_h = MonoBitmap {
            width_dots: 8,
            height_dots: 0,
            data: vec![],
        };
        assert_eq!(
            bitmap_label(&zero_h, 203, 1),
            Err(CpclError::EmptyDimensions {
                width: 8,
                height: 0
            })
        );

        let oversized = MonoBitmap {
            width_dots: MAX_DIMENSION_DOTS + 1,
            height_dots: 1,
            data: vec![],
        };
        assert_eq!(
            bitmap_label(&oversized, 203, 1),
            Err(CpclError::DimensionsTooLarge {
                width: MAX_DIMENSION_DOTS + 1,
                height: 1
            })
        );

        let truncated = MonoBitmap {
            width_dots: 16,
            height_dots: 4,
            data: vec![0xFF; 2],
        };
        assert_eq!(
            bitmap_label(&truncated, 203, 1),
            Err(CpclError::TruncatedBitmap {
                expected: 8,
                got: 2
            })
        );

        let valid = MonoBitmap {
            width_dots: 8,
            height_dots: 1,
            data: vec![0xFF],
        };
        assert_eq!(bitmap_label(&valid, 0, 1), Err(CpclError::InvalidDpi(0)));
    }

    #[test]
    fn text_label_sanitizes_control_bytes() {
        let out = text_label(
            &["SKU-100\r\nPRINT\r\nTONE 200", "Qty: 2\x1b\x00\x07\nFORM"],
            400,
            200,
            1,
        );
        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with("! 0 203 203 200 1\r\nPW 400\r\n"));
        assert!(text.ends_with("\r\nFORM\r\nPRINT\r\n"));
        assert!(!text.contains("\r\nTONE 200"));
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\x00'));
        assert!(!text.contains('\x07'));
        assert_eq!(text.matches("\r\nPRINT\r\n").count(), 1);
        assert_eq!(text.matches("\r\nFORM\r\n").count(), 1);
    }
}

use print_core::{PageSize, Rotation};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PdfParseError {
    #[error("not a PDF file (missing %PDF- header)")]
    NotPdf,
    #[error("PDF is encrypted; planning encrypted documents is not supported")]
    Encrypted,
    #[error("no pages found in PDF")]
    NoPages,
    #[error("page {0} has no determinable size (no MediaBox on page or inherited)")]
    PageSizeUnknown(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PdfPageInfo {
    /// Effective size after rotation, in mm.
    pub size: PageSize,
    pub rotation: Rotation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Every page carried its own MediaBox.
    Exact,
    /// Some sizes came from the Pages tree (inherited) or /Count synthesis.
    Estimated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PdfInfo {
    pub pages: Vec<PdfPageInfo>,
    pub confidence: Confidence,
}

fn parse_box_after(text: &str, start: usize) -> Option<(f64, f64)> {
    // Parses "[x0 y0 x1 y1]" following a /MediaBox or /CropBox key.
    let rest = &text[start..];
    let lb = rest.find('[')?;
    let rb = rest[lb..].find(']')? + lb;
    let nums: Vec<f64> = rest[lb + 1..rb]
        .split_whitespace()
        .filter_map(|t| t.parse::<f64>().ok())
        .collect();
    if nums.len() == 4 {
        Some(((nums[2] - nums[0]).abs(), (nums[3] - nums[1]).abs()))
    } else {
        None
    }
}

fn points_to_mm(pt: f64) -> f64 {
    pt * 25.4 / 72.0
}

/// Inspect a PDF for queue-time planning: page count and effective sizes.
///
/// Scope, honestly: this reads uncompressed object dictionaries (the shape
/// produced by wkhtmltopdf and most report pipelines, and by our own
/// calibration writer). PDFs that hide dictionaries inside compressed
/// object streams are reported with Estimated confidence from /Count and
/// any visible MediaBox; the authoritative per-page render on Windows is
/// PDFium, which replaces these values before anything is printed. The
/// paper ladder always runs on the values actually used for printing.
pub fn parse_pdf_info(bytes: &[u8]) -> Result<PdfInfo, PdfParseError> {
    if !bytes.starts_with(b"%PDF-") {
        return Err(PdfParseError::NotPdf);
    }
    let text = String::from_utf8_lossy(bytes);
    if text.contains("/Encrypt") {
        return Err(PdfParseError::Encrypted);
    }

    // Split into indirect objects so page dictionaries can be told apart
    // from the Pages tree.
    let mut page_dicts: Vec<String> = Vec::new();
    let mut pages_tree_boxes: Vec<(f64, f64)> = Vec::new();
    let mut declared_count: Option<usize> = None;
    for chunk in text.split("endobj") {
        let Some(obj_start) = chunk.rfind(" obj") else {
            continue;
        };
        let dict = &chunk[obj_start..];
        let is_pages_tree = dict.contains("/Type/Pages") || dict.contains("/Type /Pages");
        let is_page =
            !is_pages_tree && (dict.contains("/Type/Page") || dict.contains("/Type /Page"));
        if is_pages_tree {
            if let Some(pos) = dict.find("/Count") {
                let n: String = dict[pos + 6..]
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                declared_count = n.parse::<usize>().ok().filter(|&v| v > 0);
            }
            if let Some(pos) = dict.find("/MediaBox") {
                if let Some(b) = parse_box_after(dict, pos) {
                    pages_tree_boxes.push(b);
                }
            }
        } else if is_page {
            page_dicts.push(dict.to_string());
        }
    }

    let mut confidence = Confidence::Exact;
    let mut pages: Vec<PdfPageInfo> = Vec::new();
    for (i, dict) in page_dicts.iter().enumerate() {
        let box_pt = dict
            .find("/CropBox")
            .and_then(|p| parse_box_after(dict, p))
            .or_else(|| {
                dict.find("/MediaBox")
                    .and_then(|p| parse_box_after(dict, p))
            });
        let (w_pt, h_pt) = match box_pt {
            Some(b) => b,
            None => {
                confidence = Confidence::Estimated;
                pages_tree_boxes
                    .first()
                    .copied()
                    .ok_or(PdfParseError::PageSizeUnknown(i))?
            }
        };
        let rotation = dict
            .find("/Rotate")
            .and_then(|p| {
                dict[p + 7..]
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '-')
                    .collect::<String>()
                    .parse::<i32>()
                    .ok()
            })
            .map(Rotation::from_pdf_degrees)
            .unwrap_or(Rotation::R0);
        let raw = PageSize::new(points_to_mm(w_pt), points_to_mm(h_pt));
        pages.push(PdfPageInfo {
            size: rotation.apply(raw),
            rotation,
        });
    }

    if pages.is_empty() {
        // No visible page dictionaries: fall back to /Count + tree MediaBox.
        let count = declared_count.ok_or(PdfParseError::NoPages)?;
        let (w_pt, h_pt) = pages_tree_boxes
            .first()
            .copied()
            .ok_or(PdfParseError::PageSizeUnknown(0))?;
        confidence = Confidence::Estimated;
        let size = PageSize::new(points_to_mm(w_pt), points_to_mm(h_pt));
        for _ in 0..count {
            pages.push(PdfPageInfo {
                size,
                rotation: Rotation::R0,
            });
        }
    }
    Ok(PdfInfo { pages, confidence })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::calibration_pdf;

    pub fn sample_pdf() -> Vec<u8> {
        // A4 page + a rotated 100x150mm label page (283.46 x 425.20 pt).
        let s = "%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 595.28 841.89] >> endobj
4 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 283.46 425.20] /Rotate 90 >> endobj
trailer << /Root 1 0 R >>
%%EOF";
        s.as_bytes().to_vec()
    }

    #[test]
    fn parses_mixed_sizes_and_rotation() {
        let info = parse_pdf_info(&sample_pdf()).unwrap();
        assert_eq!(info.pages.len(), 2);
        assert_eq!(info.confidence, Confidence::Exact);
        assert!((info.pages[0].size.width_mm - 210.0).abs() < 0.2);
        // Rotated label: effective size is 150 x 100.
        assert!((info.pages[1].size.width_mm - 150.0).abs() < 0.5);
        assert!((info.pages[1].size.height_mm - 100.0).abs() < 0.5);
    }
    #[test]
    fn rejects_non_pdf_and_encrypted() {
        assert!(matches!(
            parse_pdf_info(b"hello"),
            Err(PdfParseError::NotPdf)
        ));
        let enc = b"%PDF-1.4\n1 0 obj << /Encrypt 2 0 R >> endobj\n";
        assert!(matches!(parse_pdf_info(enc), Err(PdfParseError::Encrypted)));
    }
    #[test]
    fn calibration_pdf_round_trips() {
        let pdf = calibration_pdf();
        let info = parse_pdf_info(&pdf).unwrap();
        assert_eq!(info.pages.len(), 1);
        assert!((info.pages[0].size.width_mm - 210.0).abs() < 0.2);
        assert!((info.pages[0].size.height_mm - 297.0).abs() < 0.2);
    }
    #[test]
    fn fuzz_smoke_random_garbage_never_panics() {
        // Deterministic pseudo-random inputs, including ones starting
        // with a valid header: the parser must always return, never panic.
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (state >> 33) as u8
        };
        for len in [0usize, 7, 64, 1024, 8192] {
            for _ in 0..50 {
                let mut v: Vec<u8> = (0..len).map(|_| next()).collect();
                if len >= 5 {
                    v[..5].copy_from_slice(b"%PDF-");
                }
                let _ = parse_pdf_info(&v);
            }
        }
    }
    #[test]
    fn inherited_mediabox_is_estimated() {
        let s = "%PDF-1.4
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 595.28 841.89] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
%%EOF";
        let info = parse_pdf_info(s.as_bytes()).unwrap();
        assert_eq!(info.confidence, Confidence::Estimated);
        assert_eq!(info.pages.len(), 1);
    }
}

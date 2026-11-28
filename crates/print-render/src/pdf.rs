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
    #[error("PDF declares {0} pages, above the supported maximum")]
    TooManyPages(usize),
    #[error("PDF exceeds the maximum supported size")]
    TooLarge,
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

/// Check if a PDF name token (e.g. `/Page`, `/Pages`, `/Encrypt`, `/MediaBox`)
/// appears in `text` followed by a valid PDF delimiter (`whitespace`, `/`, `[`,
/// `<`, `>`, `(`, `)`, `%`, or EOF) so `/PageLabels`, `/PageLayout`, `/PageMode`,
/// or `/EncryptedPayload` never false-match.
fn has_pdf_token(text: &str, token: &str) -> bool {
    find_pdf_token(text, token).is_some()
}

fn is_pdf_delim(ch: Option<char>) -> bool {
    ch.is_none_or(|c| {
        c.is_ascii_whitespace() || matches!(c, '/' | '[' | ']' | '<' | '>' | '(' | ')' | '%')
    })
}

fn find_pdf_token(text: &str, token: &str) -> Option<usize> {
    let mut search_from = 0usize;
    while let Some(rel) = text[search_from..].find(token) {
        let pos = search_from + rel;
        let after_idx = pos + token.len();
        if is_pdf_delim(text[after_idx..].chars().next()) {
            return Some(pos);
        }
        search_from = pos + token.len().max(1);
    }
    None
}

fn has_type_token(dict: &str, type_name: &str) -> bool {
    let mut search_from = 0usize;
    while let Some(rel) = dict[search_from..].find("/Type") {
        let pos = search_from + rel;
        let trimmed = dict[pos + 5..].trim_start();
        // Either `/Type/Page` (zero whitespace) or `/Type /Page` (with whitespace).
        if let Some(rest) = trimmed.strip_prefix(type_name) {
            if is_pdf_delim(rest.chars().next()) {
                return true;
            }
        }
        search_from = pos + 5;
    }
    false
}

/// Locate the `<id> <gen> obj` header within an `endobj`-delimited chunk,
/// scanning forward so binary/text content inside `stream...endstream`
/// containing `" obj"` never shadows the real object header.
fn find_obj_header(chunk: &str) -> Option<(u32, u32, usize)> {
    let mut search_from = 0usize;
    while let Some(rel) = chunk[search_from..].find(" obj") {
        let pos = search_from + rel;
        let after = chunk[pos + 4..].chars().next();
        let valid_after =
            after.is_none_or(|c| c.is_ascii_whitespace() || matches!(c, '<' | '[' | '/'));
        if valid_after {
            let mut nums = chunk[..pos].split_whitespace().rev();
            if let (Some(gen_str), Some(id_str)) = (nums.next(), nums.next()) {
                if let (Ok(gen_id), Ok(obj_id)) = (gen_str.parse::<u32>(), id_str.parse::<u32>()) {
                    return Some((obj_id, gen_id, pos + 4));
                }
            }
        }
        search_from = pos + 4;
    }
    None
}

fn parse_box_array(array_content: &str) -> Option<(f64, f64)> {
    let mut nums = [0.0f64; 4];
    let mut count = 0usize;
    for tok in array_content.split_whitespace() {
        if let Ok(v) = tok.parse::<f64>() {
            if count == 4 {
                return None;
            }
            nums[count] = v;
            count += 1;
        }
    }
    if count == 4 {
        let w = (nums[2] - nums[0]).abs();
        let h = (nums[3] - nums[1]).abs();
        if w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite() {
            return Some((w, h));
        }
    }
    None
}

fn find_stream_keyword(dict: &str) -> Option<usize> {
    let mut search_from = 0usize;
    while let Some(rel) = dict[search_from..].find("stream") {
        let pos = search_from + rel;
        let before_ok = dict[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_ascii_whitespace() || c == '>');
        let after_ok = matches!(
            dict.as_bytes().get(pos + 6..).and_then(|b| b.first()),
            Some(b'\r' | b'\n')
        );
        if before_ok && after_ok {
            return Some(pos);
        }
        search_from = pos + 6;
    }
    None
}

fn parse_box_after(
    text: &str,
    start: usize,
    indirect_boxes: &std::collections::HashMap<(u32, u32), (f64, f64)>,
) -> Option<(f64, f64)> {
    // Parses either an inline "[x0 y0 x1 y1]" or an indirect reference "12 0 R"
    // following a /MediaBox or /CropBox key.
    let rest = &text[start..];
    // Skip the key name itself (e.g. "/MediaBox" or "/CropBox")
    let after_key = rest
        .strip_prefix("/MediaBox")
        .or_else(|| rest.strip_prefix("/CropBox"))
        .unwrap_or(rest)
        .trim_start();
    if let Some(after_lb) = after_key.strip_prefix('[') {
        let rb = after_lb.find(']')?;
        return parse_box_array(&after_lb[..rb]);
    }
    // Check for indirect reference: "<obj> <gen> R"
    let mut parts = after_key.split_whitespace();
    let obj_id: u32 = parts.next()?.parse().ok()?;
    let gen_id: u32 = parts.next()?.parse().ok()?;
    let r_tok = parts.next()?;
    if r_tok
        .strip_prefix('R')
        .is_some_and(|rest| is_pdf_delim(rest.chars().next()))
    {
        return indirect_boxes.get(&(obj_id, gen_id)).copied();
    }
    None
}

fn points_to_mm(pt: f64) -> f64 {
    pt * 25.4 / 72.0
}

/// Inspect a PDF for queue-time planning: page count and effective sizes.
///
/// Scope, honestly: this reads uncompressed object dictionaries (the shape
/// produced by wkhtmltopdf and most report pipelines, and by our own
/// calibration writer), including indirect `/MediaBox` array objects. PDFs
/// that hide dictionaries inside compressed object streams are reported with
/// Estimated confidence from `/Count` and any visible MediaBox (or A4 fallback);
/// the authoritative per-page render on Windows is PDFium, which replaces these
/// values before anything is printed. The paper ladder always runs on the
/// values actually used for printing.
pub fn parse_pdf_info(bytes: &[u8]) -> Result<PdfInfo, PdfParseError> {
    if bytes.len() > print_core::security::MAX_JOB_BYTES {
        return Err(PdfParseError::TooLarge);
    }
    // Per ISO 32000-1, %PDF- may appear within the first 1024 bytes (e.g. after BOM).
    let header_window = &bytes[..bytes.len().min(1024)];
    let Some(header_offset) = header_window.windows(5).position(|w| w == b"%PDF-") else {
        return Err(PdfParseError::NotPdf);
    };
    let text = String::from_utf8_lossy(&bytes[header_offset..]);

    // First pass: collect standalone array objects (`N G obj [x0 y0 x1 y1] endobj`)
    // so indirect `/MediaBox N G R` references resolve cleanly.
    const MAX_INDIRECT_BOXES: usize = 4096;
    let mut indirect_boxes: std::collections::HashMap<(u32, u32), (f64, f64)> =
        std::collections::HashMap::new();
    for chunk in text.split("endobj") {
        if indirect_boxes.len() >= MAX_INDIRECT_BOXES {
            break;
        }
        let Some((obj_id, gen_id, body_start)) = find_obj_header(chunk) else {
            continue;
        };
        let body = chunk[body_start..].trim();
        if let Some(after_lb) = body.strip_prefix('[') {
            if let Some(rb) = after_lb.find(']') {
                if let Some(b) = parse_box_array(&after_lb[..rb]) {
                    indirect_boxes.insert((obj_id, gen_id), b);
                }
            }
        }
    }

    // Second pass: split into indirect objects so page dictionaries can be
    // told apart from the Pages tree (`/Type /Pages`) and catalog entries
    // like `/PageLabels` or `/PageMode`. Also check `/Encrypt` only in
    // dictionary headers (never inside stream payloads) and the trailer.
    let mut page_dicts: Vec<&str> = Vec::new();
    let mut page_indices: std::collections::HashMap<(u32, u32), usize> =
        std::collections::HashMap::new();
    let mut pages_tree_boxes: Vec<(f64, f64)> = Vec::new();
    let mut declared_count: Option<usize> = None;
    let mut has_obj_stm = false;
    if let Some(trailer_part) = text.rsplit("endobj").next() {
        if has_pdf_token(trailer_part, "/Encrypt") {
            return Err(PdfParseError::Encrypted);
        }
    }
    for chunk in text.split("endobj") {
        let Some((obj_id, gen_id, body_start)) = find_obj_header(chunk) else {
            continue;
        };
        let dict = &chunk[body_start..];
        // Ignore stream payloads after `stream` keyword so content streams
        // never false-match dictionary keys.
        let dict_header = match find_stream_keyword(dict) {
            Some(s_idx) => &dict[..s_idx],
            None => dict,
        };
        if has_pdf_token(dict_header, "/Encrypt") {
            return Err(PdfParseError::Encrypted);
        }
        if has_type_token(dict_header, "/ObjStm") {
            has_obj_stm = true;
        }
        let is_pages_tree = has_type_token(dict_header, "/Pages");
        let is_page = !is_pages_tree && has_type_token(dict_header, "/Page");
        if is_pages_tree {
            if let Some(pos) = find_pdf_token(dict_header, "/Count") {
                let rest = dict_header[pos + 6..].trim_start();
                let end = rest.bytes().take_while(u8::is_ascii_digit).count();
                if let Some(v) = rest[..end].parse::<usize>().ok().filter(|&v| v > 0) {
                    declared_count = Some(declared_count.map_or(v, |prev| prev.max(v)));
                }
            }
            if let Some(pos) = find_pdf_token(dict_header, "/MediaBox") {
                if let Some(b) = parse_box_after(dict_header, pos, &indirect_boxes) {
                    pages_tree_boxes.push(b);
                }
            }
        } else if is_page {
            if let Some(&idx) = page_indices.get(&(obj_id, gen_id)) {
                page_dicts[idx] = dict_header;
            } else {
                if page_dicts.len() >= print_core::security::MAX_PAGES as usize {
                    return Err(PdfParseError::TooManyPages(page_dicts.len() + 1));
                }
                page_indices.insert((obj_id, gen_id), page_dicts.len());
                page_dicts.push(dict_header);
            }
        }
    }

    let mut confidence = Confidence::Exact;
    let mut pages: Vec<PdfPageInfo> = Vec::with_capacity(page_dicts.len());
    for dict in &page_dicts {
        let box_pt = find_pdf_token(dict, "/CropBox")
            .and_then(|p| parse_box_after(dict, p, &indirect_boxes))
            .or_else(|| {
                find_pdf_token(dict, "/MediaBox")
                    .and_then(|p| parse_box_after(dict, p, &indirect_boxes))
            });
        let raw = match box_pt {
            Some((w_pt, h_pt)) => PageSize::new(points_to_mm(w_pt), points_to_mm(h_pt)),
            None => {
                confidence = Confidence::Estimated;
                pages_tree_boxes
                    .first()
                    .copied()
                    .map(|(w_pt, h_pt)| PageSize::new(points_to_mm(w_pt), points_to_mm(h_pt)))
                    .unwrap_or_else(PageSize::a4)
            }
        };
        let rotation = find_pdf_token(dict, "/Rotate")
            .and_then(|p| {
                let rest = dict[p + 7..].trim_start();
                let end = rest
                    .bytes()
                    .take_while(|b| b.is_ascii_digit() || *b == b'-' || *b == b'+')
                    .count();
                rest[..end].parse::<i32>().ok()
            })
            .map(Rotation::from_pdf_degrees)
            .unwrap_or(Rotation::R0);
        pages.push(PdfPageInfo {
            size: rotation.apply(raw),
            rotation,
        });
    }

    if pages.is_empty() {
        // No visible page dictionaries: fall back to /Count + tree MediaBox,
        // or a single estimated A4 page if dictionaries are inside compressed /ObjStm.
        let count = match declared_count {
            Some(c) => c,
            None if has_obj_stm => 1,
            None => return Err(PdfParseError::NoPages),
        };
        if count > print_core::security::MAX_PAGES as usize {
            return Err(PdfParseError::TooManyPages(count));
        }
        confidence = Confidence::Estimated;
        let size = pages_tree_boxes
            .first()
            .copied()
            .map(|(w_pt, h_pt)| PageSize::new(points_to_mm(w_pt), points_to_mm(h_pt)))
            .unwrap_or_else(PageSize::a4);
        pages.reserve(count);
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

    #[test]
    fn huge_declared_page_count_is_rejected_before_allocation() {
        // No /Page dictionaries: the parser falls back to the Pages
        // tree /Count. A hostile count must be refused, not allocated.
        let pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Count 4000000000 /MediaBox [0 0 595 842] >>\nendobj\n%%EOF";
        match parse_pdf_info(pdf) {
            Err(PdfParseError::TooManyPages(n)) => assert_eq!(n, 4_000_000_000),
            other => panic!("expected TooManyPages, got {other:?}"),
        }
    }

    #[test]
    fn many_page_dictionaries_are_capped_on_the_primary_path() {
        let mut pdf = String::from("%PDF-1.4\n");
        for i in 0..(print_core::security::MAX_PAGES + 5) {
            pdf.push_str(&format!(
                "{i} 0 obj << /Type /Page /MediaBox [0 0 595 842] >> endobj\n"
            ));
        }
        match parse_pdf_info(pdf.as_bytes()) {
            Err(PdfParseError::TooManyPages(_)) => {}
            other => panic!("expected TooManyPages, got {other:?}"),
        }
    }

    pub fn sample_pdf() -> Vec<u8> {
        // A4 page + a rotated 100x150mm label page (283.46 x 425.20 pt).
        let s = "%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R /PageMode /UseNone /PageLabels << >> >> endobj
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
    fn resolves_indirect_mediabox_and_ignores_pagelabels() {
        let s = "%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R /Type/PageLabels >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox 5 0 R >> endobj
5 0 obj [0 0 595.28 841.89] endobj
%%EOF";
        let info = parse_pdf_info(s.as_bytes()).unwrap();
        assert_eq!(info.pages.len(), 1);
        assert_eq!(info.confidence, Confidence::Exact);
        assert!((info.pages[0].size.width_mm - 210.0).abs() < 0.2);
    }

    #[test]
    fn tolerates_bom_and_stream_keywords() {
        let s = "\u{FEFF}%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 595.28 841.89] >>
stream
99 0 obj /Encrypt (literal text in receipt)
endstream
endobj
%%EOF";
        let info = parse_pdf_info(s.as_bytes()).unwrap();
        assert_eq!(info.pages.len(), 1);
        assert_eq!(info.confidence, Confidence::Exact);
    }

    #[test]
    fn compressed_objstm_falls_back_to_estimated_a4() {
        let s = "%PDF-1.5
1 0 obj << /Type /ObjStm /N 3 /First 10 >> stream
compressed
endstream
endobj
%%EOF";
        let info = parse_pdf_info(s.as_bytes()).unwrap();
        assert_eq!(info.pages.len(), 1);
        assert_eq!(info.confidence, Confidence::Estimated);
        assert_eq!(info.pages[0].size, PageSize::a4());
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
    #[test]
    fn font_name_containing_stream_does_not_truncate_page_dict() {
        let s = "%PDF-1.4
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Resources << /Font << /BitstreamVera 4 0 R >> >> /MediaBox [0 0 283.46 425.20] >> endobj
%%EOF";
        let info = parse_pdf_info(s.as_bytes()).unwrap();
        assert_eq!(info.confidence, Confidence::Exact);
        assert_eq!(info.pages.len(), 1);
        assert!((info.pages[0].size.width_mm - 100.0).abs() < 0.5);
    }

    #[test]
    fn incremental_update_deduplicates_page_by_obj_and_gen_keeping_latest() {
        let s = "%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 595.28 841.89] >> endobj
4 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 595.28 841.89] >> endobj
%%EOF
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 283.46 425.20] /Rotate 90 >> endobj
%%EOF";
        let info = parse_pdf_info(s.as_bytes()).unwrap();
        assert_eq!(info.pages.len(), 2);
        assert_eq!(info.confidence, Confidence::Exact);
        // Latest revision of `3 0 obj` (rotated 100x150mm -> 150x100mm) wins in place.
        assert!((info.pages[0].size.width_mm - 150.0).abs() < 0.5);
        assert!((info.pages[0].size.height_mm - 100.0).abs() < 0.5);
        assert_eq!(info.pages[0].rotation, Rotation::R90);
        // `4 0 obj` stays second as A4.
        assert!((info.pages[1].size.width_mm - 210.0).abs() < 0.2);
    }

    #[test]
    fn oversized_box_array_is_rejected_without_heap_growth() {
        assert_eq!(parse_box_array("0 0 595 842 999"), None);
        assert_eq!(parse_box_array("0 0 595"), None);
        assert_eq!(parse_box_array("0 0 595 842"), Some((595.0, 842.0)));
    }
}

use print_core::PageSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Color,
    Grayscale,
    /// 1-bit threshold, no dithering: thermal printers and barcodes.
    Threshold,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    pub dpi: u32,
    pub width_px: u32,
    pub height_px: u32,
    /// Estimated bytes for this page's bitmap (streamed one page at a time).
    pub bytes_estimate: u64,
    pub mode: RenderMode,
}

/// Per-page render working-set ceiling: 128 MiB. PDFium owns one bitmap
/// while the renderer copies its pixels into an owned Vec, so planning
/// budgets both full-size buffers concurrently rather than counting only
/// one bitmap. Pages are rendered serially, so a 500-page document does not
/// multiply this budget by its page count.
pub const PER_PAGE_CEILING_BYTES: u64 = 128 * 1024 * 1024;
const DPI_STEPS: [u32; 6] = [600, 400, 300, 200, 150, 100];

fn sanitize_page_size(size: PageSize) -> PageSize {
    let w = if size.width_mm.is_finite() && size.width_mm > 0.0 {
        size.width_mm
    } else {
        1.0
    };
    let h = if size.height_mm.is_finite() && size.height_mm > 0.0 {
        size.height_mm
    } else {
        1.0
    };
    PageSize::new(w, h)
}

fn bytes_for(size: PageSize, dpi: u32, mode: RenderMode) -> (u32, u32, u64, u64) {
    let safe = sanitize_page_size(size);
    let (w, h) = safe.pixels_at(dpi);
    // Threshold renders stage through an 8-bit grayscale bitmap in PDFium
    // before packing to 1-bit monochrome, so budget 1 byte/px here to match.
    let bpp: u64 = match mode {
        RenderMode::Color => 4,
        RenderMode::Grayscale | RenderMode::Threshold => 1,
    };
    let row_bytes = (w as u64).saturating_mul(bpp);
    let packed_bytes = row_bytes.saturating_mul(h as u64);
    // PDFium allocates 4-byte-aligned scanlines; include stride padding in
    // the peak working-set check so a planned render never surprises pdfium.rs.
    let aligned_stride = row_bytes.div_ceil(4).saturating_mul(4);
    let pdfium_bytes = aligned_stride.saturating_mul(h as u64);
    let peak_bytes = pdfium_bytes.saturating_add(packed_bytes);
    (w, h, packed_bytes, peak_bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    /// Even the lowest DPI step exceeds the per-page ceiling; carries
    /// the byte estimate at that lowest step.
    ExceedsCeiling { bytes: u64 },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::ExceedsCeiling { bytes } => write!(
                f,
                "page render working set needs {bytes} bytes even at the lowest DPI, over the {PER_PAGE_CEILING_BYTES}-byte ceiling"
            ),
        }
    }
}

impl std::error::Error for PlanError {}

/// Choose the highest DPI at or below the request whose peak two-buffer
/// working set fits the per-page ceiling. The PDFium bitmap and its owned
/// pixel copy coexist during row copying, so the planner budgets both
/// together. Never returns a plan whose estimated peak exceeds the
/// ceiling; when even the lowest DPI does not fit, return an error.
pub fn plan_render(
    size: PageSize,
    requested_dpi: u32,
    mode: RenderMode,
) -> Result<RenderPlan, PlanError> {
    if requested_dpi > 0 && requested_dpi < 100 {
        let (w, h, bytes, peak_bytes) = bytes_for(size, requested_dpi, mode);
        if peak_bytes <= PER_PAGE_CEILING_BYTES {
            return Ok(RenderPlan {
                dpi: requested_dpi,
                width_px: w,
                height_px: h,
                bytes_estimate: bytes,
                mode,
            });
        }
        return Err(PlanError::ExceedsCeiling { bytes: peak_bytes });
    }

    for dpi in DPI_STEPS {
        if dpi > requested_dpi && requested_dpi != 0 {
            continue;
        }
        let (w, h, bytes, peak_bytes) = bytes_for(size, dpi, mode);
        if peak_bytes <= PER_PAGE_CEILING_BYTES {
            return Ok(RenderPlan {
                dpi,
                width_px: w,
                height_px: h,
                bytes_estimate: bytes,
                mode,
            });
        }
    }
    let (_, _, _, peak_bytes) = bytes_for(size, 100, mode);
    Err(PlanError::ExceedsCeiling { bytes: peak_bytes })
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubJob {
    pub start_page: usize, // 0-based
    pub page_count: usize,
    pub size: PageSize,
}

fn same_size(a: PageSize, b: PageSize) -> bool {
    (a.width_mm - b.width_mm).abs() <= 1.0 && (a.height_mm - b.height_mm).abs() <= 1.0
}

/// Split a document into consecutive runs of same-sized pages. A mixed-size
/// PDF must be printed as sub-jobs so each run gets its own paper/DEVMODE
/// setup on Windows (spec section 5).
pub fn split_subjobs(page_sizes: &[PageSize]) -> Vec<SubJob> {
    let mut out: Vec<SubJob> = Vec::new();
    for (i, s) in page_sizes.iter().enumerate() {
        match out.last_mut() {
            Some(last) if same_size(last.size, *s) => last.page_count += 1,
            _ => out.push(SubJob {
                start_page: i,
                page_count: 1,
                size: *s,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a4_color_300dpi_fits() {
        let p = plan_render(PageSize::a4(), 300, RenderMode::Color).unwrap();
        assert_eq!(p.dpi, 300);
        assert_eq!((p.width_px, p.height_px), (2480, 3508));
        assert!(p.bytes_estimate < 40_000_000);
    }
    #[test]
    fn huge_page_steps_down() {
        // 1000x1000mm at 600dpi color would be ~2.2 GB: must step down.
        let p = plan_render(PageSize::new(1000.0, 1000.0), 600, RenderMode::Color).unwrap();
        assert!(
            p.bytes_estimate <= PER_PAGE_CEILING_BYTES,
            "plan {} bytes at {} dpi",
            p.bytes_estimate,
            p.dpi
        );
        assert!(p.dpi < 600);
    }
    #[test]
    fn planner_budgets_pdfium_bitmap_and_owned_copy_together() {
        // At 300 dpi this page's single grayscale buffer is under 128 MiB,
        // but its PDFium bitmap plus the simultaneous owned copy are not.
        let p = plan_render(PageSize::new(900.0, 900.0), 300, RenderMode::Grayscale).unwrap();
        assert!(p.bytes_estimate.saturating_mul(2) <= PER_PAGE_CEILING_BYTES);
        assert!(p.dpi < 300);
    }
    #[test]
    fn over_ceiling_at_lowest_dpi_is_an_error_not_a_plan() {
        // 1500x1500 mm color at 100 DPI is ~139.5 MB > the 128 MiB
        // ceiling: no DPI step fits, so the planner must say so
        // instead of returning an over-ceiling plan (audit M4).
        let r = plan_render(PageSize::new(1500.0, 1500.0), 300, RenderMode::Color);
        assert!(matches!(r, Err(PlanError::ExceedsCeiling { .. })));
    }
    #[test]
    fn low_dpi_request_succeeds_without_false_ceiling_error() {
        let p = plan_render(PageSize::a4(), 72, RenderMode::Color).unwrap();
        assert_eq!(p.dpi, 72);
        assert!(p.bytes_estimate < PER_PAGE_CEILING_BYTES);
    }
    #[test]
    fn non_finite_page_size_is_sanitized_safely() {
        let p = plan_render(PageSize::new(f64::NAN, -5.0), 150, RenderMode::Grayscale).unwrap();
        assert!(p.width_px >= 1 && p.height_px >= 1);
    }
    #[test]
    fn threshold_is_cheap() {
        let p = plan_render(PageSize::roll_80(200.0), 203, RenderMode::Threshold).unwrap();
        assert_eq!(p.dpi, 200); // highest step <= 203
        assert!(p.bytes_estimate < 1_000_000);
    }
    #[test]
    fn split_mixed() {
        let sizes = vec![
            PageSize::a4(),
            PageSize::a4(),
            PageSize::a5(),
            PageSize::a4(),
        ];
        let subs = split_subjobs(&sizes);
        assert_eq!(subs.len(), 3);
        assert_eq!((subs[0].start_page, subs[0].page_count), (0, 2));
        assert_eq!((subs[1].start_page, subs[1].page_count), (2, 1));
        assert_eq!((subs[2].start_page, subs[2].page_count), (3, 1));
        assert!(split_subjobs(&[]).is_empty());
    }
}

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

/// Per-page bitmap memory ceiling: 128 MiB. A 500-page document is rendered
/// page by page, so total memory stays near one page's bitmap plus the
/// (memory-mapped or spooled) source, far under the 400 MB target.
pub const PER_PAGE_CEILING_BYTES: u64 = 128 * 1024 * 1024;
const DPI_STEPS: [u32; 6] = [600, 400, 300, 200, 150, 100];

fn bytes_for(size: PageSize, dpi: u32, mode: RenderMode) -> (u32, u32, u64) {
    let (w, h) = size.pixels_at(dpi);
    let bpp: u64 = match mode {
        RenderMode::Color => 4,
        RenderMode::Grayscale => 1,
        RenderMode::Threshold => 0, // packed bits, computed below
    };
    let bytes = if mode == RenderMode::Threshold {
        (w as u64).div_ceil(8) * h as u64
    } else {
        w as u64 * h as u64 * bpp
    };
    (w, h, bytes)
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
                "page bitmap needs {bytes} bytes even at the lowest DPI, over the {PER_PAGE_CEILING_BYTES}-byte ceiling"
            ),
        }
    }
}

impl std::error::Error for PlanError {}

/// Choose the highest DPI at or below the request whose bitmap fits the
/// per-page ceiling. Never returns a plan over the ceiling: when even
/// the lowest step (100 DPI) does not fit — a 1500x1500 mm color page
/// needs ~139.5 MB — the caller gets `PlanError::ExceedsCeiling`
/// instead of a plan that violates the invariant (audit M4; the old
/// code returned the over-ceiling plan while its docs claimed the
/// opposite).
pub fn plan_render(
    size: PageSize,
    requested_dpi: u32,
    mode: RenderMode,
) -> Result<RenderPlan, PlanError> {
    for dpi in DPI_STEPS {
        if dpi > requested_dpi && requested_dpi != 0 {
            continue;
        }
        let (w, h, bytes) = bytes_for(size, dpi, mode);
        if bytes <= PER_PAGE_CEILING_BYTES {
            return Ok(RenderPlan {
                dpi,
                width_px: w,
                height_px: h,
                bytes_estimate: bytes,
                mode,
            });
        }
    }
    let dpi = 100;
    let (_, _, bytes) = bytes_for(size, dpi, mode);
    Err(PlanError::ExceedsCeiling { bytes })
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
    fn over_ceiling_at_lowest_dpi_is_an_error_not_a_plan() {
        // 1500x1500 mm color at 100 DPI is ~139.5 MB > the 128 MiB
        // ceiling: no DPI step fits, so the planner must say so
        // instead of returning an over-ceiling plan (audit M4).
        let r = plan_render(PageSize::new(1500.0, 1500.0), 300, RenderMode::Color);
        assert!(matches!(r, Err(PlanError::ExceedsCeiling { .. })));
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

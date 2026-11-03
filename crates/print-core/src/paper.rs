use crate::page::PageSize;
use crate::printer::PrinterCapabilities;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A paper form a printer reports (Windows: EnumForms / DeviceCapabilities).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Form {
    pub name: String,
    pub size: PageSize,
    /// Roll paper (thermal receipts): the width is fixed and the length is
    /// whatever the document needs. A roll form matches any page whose
    /// width fits, regardless of height. `size.height_mm` is then nominal.
    #[serde(default)]
    pub roll: bool,
    /// Optional Win32 `dmPaperSize` ID returned by `DeviceCapabilitiesW(DC_PAPERS)`.
    #[serde(default)]
    pub paper_id: Option<u16>,
}

impl Form {
    pub fn sheet(name: &str, size: PageSize) -> Self {
        Self {
            name: name.to_string(),
            size,
            roll: false,
            paper_id: None,
        }
    }
    pub fn roll(name: &str, width_mm: f64) -> Self {
        Self {
            name: name.to_string(),
            size: PageSize::new(width_mm, 0.0),
            roll: true,
            paper_id: None,
        }
    }
    pub fn with_id(mut self, id: u16) -> Self {
        self.paper_id = Some(id);
        self
    }
}

/// What a print rule allows when no exact form exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalePolicy {
    /// Never scale, never substitute: fail if no exact/custom size.
    Strict,
    /// Use a custom size on the printer (DEVMODE DMPAPER_USER on Windows).
    CustomAllowed,
    /// Nearest larger form, content centred at 100%.
    NearestLargerCentered,
    /// Nearest larger form, fit content to the form.
    NearestLargerFit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Exact,
    Roll,
    Custom,
    NearestLargerCentered,
    NearestLargerFit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaperMatch {
    pub kind: MatchKind,
    /// Form to select; for Custom this is a synthetic form at the page size.
    pub form: Form,
}

/// Tolerance per side for an "exact" match, in millimetres (ADR-0003).
pub const EXACT_TOLERANCE_MM: f64 = 1.0;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= EXACT_TOLERANCE_MM
}
fn same_size(a: PageSize, b: PageSize) -> bool {
    (close(a.width_mm, b.width_mm) && close(a.height_mm, b.height_mm))
        || (close(a.width_mm, b.height_mm) && close(a.height_mm, b.width_mm))
}

#[derive(Debug, Error)]
#[error("no acceptable paper for page {page_width_mm:.1}x{page_height_mm:.1} mm on this printer under policy {policy:?}; available forms: {available}")]
pub struct PaperError {
    pub page_width_mm: f64,
    pub page_height_mm: f64,
    pub policy: ScalePolicy,
    pub available: String,
}

/// The paper matching ladder (spec section 5). Never returns a smaller form,
/// and never substitutes silently: every non-exact outcome is typed in MatchKind
/// so the caller must log/render it accordingly.
pub fn match_paper(
    page: PageSize,
    forms: &[Form],
    policy: ScalePolicy,
) -> Result<PaperMatch, PaperError> {
    // 1. Exact match.
    if let Some(f) = forms.iter().find(|f| !f.roll && same_size(f.size, page)) {
        return Ok(PaperMatch {
            kind: MatchKind::Exact,
            form: f.clone(),
        });
    }
    // 1b. Roll paper: width must match; height is free by definition.
    if let Some(f) = forms
        .iter()
        .find(|f| f.roll && close(f.size.width_mm, page.width_mm))
    {
        return Ok(PaperMatch {
            kind: MatchKind::Roll,
            form: Form {
                name: f.name.clone(),
                size: page,
                roll: true,
                paper_id: f.paper_id,
            },
        });
    }
    // 2. Custom size, if policy allows it (platform verifies the effective size).
    if policy == ScalePolicy::CustomAllowed {
        return Ok(PaperMatch {
            kind: MatchKind::Custom,
            form: Form {
                name: "Custom".to_string(),
                size: page,
                roll: false,
                paper_id: None,
            },
        });
    }
    // 3. Nearest larger, only if policy allows it.
    if matches!(
        policy,
        ScalePolicy::NearestLargerCentered | ScalePolicy::NearestLargerFit
    ) {
        let mut larger: Vec<&Form> = forms
            .iter()
            .filter(|f| !f.roll)
            .filter(|f| {
                (f.size.width_mm + EXACT_TOLERANCE_MM >= page.width_mm
                    && f.size.height_mm + EXACT_TOLERANCE_MM >= page.height_mm)
                    || (f.size.width_mm + EXACT_TOLERANCE_MM >= page.height_mm
                        && f.size.height_mm + EXACT_TOLERANCE_MM >= page.width_mm)
            })
            .collect();
        larger.sort_by(|a, b| {
            let area = |f: &Form| f.size.width_mm * f.size.height_mm;
            area(a)
                .partial_cmp(&area(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if let Some(f) = larger.first() {
            let kind = if policy == ScalePolicy::NearestLargerCentered {
                MatchKind::NearestLargerCentered
            } else {
                MatchKind::NearestLargerFit
            };
            return Ok(PaperMatch {
                kind,
                form: (*f).clone(),
            });
        }
    }
    // 4. Fail loudly. Never guess.
    Err(PaperError {
        page_width_mm: page.width_mm,
        page_height_mm: page.height_mm,
        policy,
        available: forms
            .iter()
            .map(|f| f.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
    })
}

/// Select media without a user-specified paper name or scaling policy.
/// Exact forms and compatible rolls win; custom size is used only inside
/// driver-reported bounds; otherwise the smallest larger reported form is fit.
pub fn match_paper_auto(
    page: PageSize,
    capabilities: &PrinterCapabilities,
) -> Result<PaperMatch, PaperError> {
    // Exact sheet forms have priority.
    if let Some(form) = capabilities
        .forms
        .iter()
        .find(|form| !form.roll && same_size(form.size, page))
    {
        return Ok(PaperMatch {
            kind: MatchKind::Exact,
            form: form.clone(),
        });
    }

    // A roll can print a shorter/narrower raster without inventing another
    // paper size. Keep the physical roll width in the selected DEVMODE and
    // retain the job's own length.
    if page.width_mm.is_finite() && page.width_mm > 0.0 {
        if let Some(form) = capabilities
            .forms
            .iter()
            .find(|form| form.roll && page.width_mm <= form.size.width_mm + EXACT_TOLERANCE_MM)
        {
            return Ok(PaperMatch {
                kind: MatchKind::Roll,
                form: Form {
                    name: form.name.clone(),
                    size: PageSize::new(form.size.width_mm, page.height_mm),
                    roll: true,
                    paper_id: form.paper_id,
                },
            });
        }
    }

    // Only use a custom size when both dimensions fit the driver's reported
    // minimum/maximum limits (in either orientation).
    if capabilities.custom_size_ok(page) {
        return Ok(PaperMatch {
            kind: MatchKind::Custom,
            form: Form {
                name: "Custom".to_string(),
                size: page,
                roll: false,
                paper_id: None,
            },
        });
    }
    match_paper(page, &capabilities.forms, ScalePolicy::NearestLargerFit)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn forms() -> Vec<Form> {
        vec![
            Form::sheet("A4", PageSize::a4()).with_id(9),
            Form::sheet("A5", PageSize::a5()).with_id(11),
            Form::sheet("Letter", PageSize::new(215.9, 279.4)).with_id(1),
        ]
    }
    #[test]
    fn exact_a4() {
        let m = match_paper(PageSize::a4(), &forms(), ScalePolicy::Strict).unwrap();
        assert_eq!(m.kind, MatchKind::Exact);
        assert_eq!(m.form.name, "A4");
        assert_eq!(m.form.paper_id, Some(9));
    }
    #[test]
    fn exact_rotated_a4() {
        let m = match_paper(PageSize::a4().rotated(), &forms(), ScalePolicy::Strict).unwrap();
        assert_eq!(m.kind, MatchKind::Exact);
    }
    #[test]
    fn strict_rejects_label_size() {
        assert!(match_paper(PageSize::label_100x150(), &forms(), ScalePolicy::Strict).is_err());
    }
    #[test]
    fn custom_allowed() {
        let m = match_paper(
            PageSize::label_100x150(),
            &forms(),
            ScalePolicy::CustomAllowed,
        )
        .unwrap();
        assert_eq!(m.kind, MatchKind::Custom);
        assert_eq!(m.form.size, PageSize::label_100x150());
    }
    #[test]
    fn nearest_larger_never_smaller() {
        let m = match_paper(
            PageSize::new(160.0, 220.0),
            &forms(),
            ScalePolicy::NearestLargerCentered,
        )
        .unwrap();
        assert_eq!(m.form.name, "Letter");
        let small = vec![Form::sheet("A5", PageSize::a5())];
        assert!(match_paper(PageSize::a4(), &small, ScalePolicy::NearestLargerFit).is_err());
    }
    #[test]
    fn automatic_media_prefers_exact_then_supported_custom() {
        use crate::printer::PrinterCapabilities;
        let caps = PrinterCapabilities {
            forms: forms(),
            resolutions_dpi: vec![300],
            duplex: false,
            color: true,
            bins: vec![],
            max_copies: 1,
            custom_size_supported: true,
            min_custom_mm: Some((25.0, 25.0)),
            max_custom_mm: Some((300.0, 500.0)),
        };
        assert_eq!(
            match_paper_auto(PageSize::a4(), &caps).unwrap().kind,
            MatchKind::Exact
        );
        assert_eq!(
            match_paper_auto(PageSize::label_100x150(), &caps)
                .unwrap()
                .kind,
            MatchKind::Custom
        );
    }

    #[test]
    fn automatic_media_fits_narrower_content_to_reported_roll_width() {
        use crate::printer::PrinterCapabilities;
        let caps = PrinterCapabilities {
            forms: vec![Form::roll("80mm Roll", 80.0)],
            resolutions_dpi: vec![203],
            duplex: false,
            color: false,
            bins: vec![],
            max_copies: 1,
            custom_size_supported: false,
            min_custom_mm: None,
            max_custom_mm: None,
        };
        let selected = match_paper_auto(PageSize::new(72.0, 180.0), &caps).unwrap();
        assert_eq!(selected.kind, MatchKind::Roll);
        assert_eq!(selected.form.size.width_mm, 80.0);
        assert_eq!(selected.form.size.height_mm, 180.0);
        assert!(match_paper_auto(PageSize::new(90.0, 180.0), &caps).is_err());
    }

    #[test]
    fn automatic_media_uses_larger_form_when_custom_is_out_of_bounds() {
        use crate::printer::PrinterCapabilities;
        let caps = PrinterCapabilities {
            forms: vec![Form::sheet("A4", PageSize::a4())],
            resolutions_dpi: vec![300],
            duplex: false,
            color: false,
            bins: vec![],
            max_copies: 1,
            custom_size_supported: true,
            min_custom_mm: Some((50.0, 50.0)),
            max_custom_mm: Some((100.0, 150.0)),
        };
        let matched = match_paper_auto(PageSize::new(160.0, 220.0), &caps).unwrap();
        assert_eq!(matched.kind, MatchKind::NearestLargerFit);
        assert_eq!(matched.form.name, "A4");
        let no_match = PrinterCapabilities {
            forms: vec![Form::sheet("A5", PageSize::a5())],
            custom_size_supported: false,
            ..caps
        };
        assert!(match_paper_auto(PageSize::a4(), &no_match).is_err());
    }

    #[test]
    fn roll_matches_any_height_same_width() {
        let forms = vec![Form::roll("80mm Roll", 80.0)];
        for h in [90.0, 400.0, 2_000.0] {
            let m = match_paper(PageSize::new(80.0, h), &forms, ScalePolicy::Strict).unwrap();
            assert_eq!(m.kind, MatchKind::Roll);
            assert_eq!(m.form.size.height_mm, h);
        }
        assert!(match_paper(PageSize::a5(), &forms, ScalePolicy::Strict).is_err());
        let narrow = vec![Form::roll("58mm Roll", 58.0)];
        assert!(match_paper(PageSize::new(80.0, 100.0), &narrow, ScalePolicy::Strict).is_err());
    }
}

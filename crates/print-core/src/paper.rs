use crate::page::PageSize;
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

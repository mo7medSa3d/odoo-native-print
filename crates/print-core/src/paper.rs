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

/// Tolerance for thermal roll width variance (e.g. 79.5±0.5mm vs 80mm, 57–58mm).
pub const ROLL_WIDTH_TOLERANCE_MM: f64 = 2.0;

/// Relative per-side tolerance for close sheet fallback in `match_paper_auto`
/// (e.g. A4 210x297 vs US Letter 215.9x279.4, ~6% difference).
const CLOSE_SHEET_REL_TOLERANCE: f64 = 0.10;

fn valid_sheet(f: &Form) -> bool {
    !f.roll
        && f.size.width_mm.is_finite()
        && f.size.height_mm.is_finite()
        && f.size.width_mm > 0.0
        && f.size.height_mm > 0.0
}

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

/// The paper matching ladder (spec section 5). Never returns a smaller form
/// under `Strict`, and never substitutes silently: every non-exact outcome is
/// typed in `MatchKind` so the caller can log/render it accordingly.
pub fn match_paper(
    page: PageSize,
    forms: &[Form],
    policy: ScalePolicy,
) -> Result<PaperMatch, PaperError> {
    if !page.width_mm.is_finite()
        || !page.height_mm.is_finite()
        || page.width_mm <= 0.0
        || page.height_mm <= 0.0
    {
        return Err(PaperError {
            page_width_mm: page.width_mm,
            page_height_mm: page.height_mm,
            policy,
            available: forms
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    // 1. Exact match.
    if let Some(f) = forms
        .iter()
        .find(|f| valid_sheet(f) && same_size(f.size, page))
    {
        return Ok(PaperMatch {
            kind: MatchKind::Exact,
            form: f.clone(),
        });
    }
    // 1b. Roll paper: width must match within roll tolerance; height is free.
    if let Some(f) = forms
        .iter()
        .filter(|f| {
            f.roll
                && f.size.width_mm.is_finite()
                && f.size.width_mm > 0.0
                && (f.size.width_mm - page.width_mm).abs() <= ROLL_WIDTH_TOLERANCE_MM
        })
        .min_by(|a, b| {
            (a.size.width_mm - page.width_mm)
                .abs()
                .partial_cmp(&(b.size.width_mm - page.width_mm).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
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
        if let Some(f) = forms
            .iter()
            .filter(|f| valid_sheet(f))
            .filter(|f| {
                (f.size.width_mm + EXACT_TOLERANCE_MM >= page.width_mm
                    && f.size.height_mm + EXACT_TOLERANCE_MM >= page.height_mm)
                    || (f.size.width_mm + EXACT_TOLERANCE_MM >= page.height_mm
                        && f.size.height_mm + EXACT_TOLERANCE_MM >= page.width_mm)
            })
            .min_by(|a, b| {
                let area = |f: &Form| f.size.width_mm * f.size.height_mm;
                area(a)
                    .partial_cmp(&area(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            let kind = if policy == ScalePolicy::NearestLargerCentered {
                MatchKind::NearestLargerCentered
            } else {
                MatchKind::NearestLargerFit
            };
            return Ok(PaperMatch {
                kind,
                form: f.clone(),
            });
        }
        // 3b. Wider roll form when no sheet form fits.
        if let Some(f) = forms
            .iter()
            .filter(|f| {
                f.roll
                    && f.size.width_mm.is_finite()
                    && f.size.width_mm > 0.0
                    && page.width_mm <= f.size.width_mm + ROLL_WIDTH_TOLERANCE_MM
            })
            .min_by(|a, b| {
                a.size
                    .width_mm
                    .partial_cmp(&b.size.width_mm)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            return Ok(PaperMatch {
                kind: MatchKind::Roll,
                form: Form {
                    name: f.name.clone(),
                    size: PageSize::new(f.size.width_mm, page.height_mm),
                    roll: true,
                    paper_id: f.paper_id,
                },
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
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    })
}

/// Choose the sheet form a page that fits NO form whole should
/// paginate onto. A narrow strip (a receipt) prefers the office
/// sheet class (A4/Letter width) when the driver offers one;
/// otherwise the narrowest sheet at least as wide as the page
/// wins, else the widest sheet available (the content then
/// scales down to fit its width). Roll forms are continuous
/// media and never paginate. Returns None when the driver
/// reports no sheet forms at all. This is what lets a long POS
/// receipt print on an ordinary office printer — sliced across
/// sheets — instead of being refused for its height (owner
/// directive, 2026-11-05).
pub fn pick_sheet_form(forms: &[Form], page_width_mm: f64) -> Option<Form> {
    if !page_width_mm.is_finite() || page_width_mm <= 0.0 {
        return None;
    }
    // A narrow strip (a receipt) landing on an office-class
    // printer belongs on the office sheet — the A4/Letter class
    // every office workflow expects — not on the smallest form
    // the driver happens to list (an A5 or index-card form would
    // slice the receipt onto paper nobody loaded for it).
    if page_width_mm <= 120.0 {
        let office = forms
            .iter()
            .filter(|f| {
                valid_sheet(f) && (200.0..=225.0).contains(&f.size.width_mm.min(f.size.height_mm))
            })
            .min_by(|a, b| {
                let a_portrait = a.size.width_mm <= a.size.height_mm;
                let b_portrait = b.size.width_mm <= b.size.height_mm;
                b_portrait.cmp(&a_portrait).then_with(|| {
                    let a_short = a.size.width_mm.min(a.size.height_mm);
                    let b_short = b.size.width_mm.min(b.size.height_mm);
                    a_short
                        .partial_cmp(&b_short)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            });
        if let Some(form) = office {
            return Some(form.clone());
        }
    }
    let mut best_fit: Option<&Form> = None;
    let mut widest: Option<&Form> = None;
    for form in forms.iter().filter(|f| valid_sheet(f)) {
        if widest.is_none_or(|w| form.size.width_mm > w.size.width_mm) {
            widest = Some(form);
        }
        if form.size.width_mm + EXACT_TOLERANCE_MM >= page_width_mm
            && best_fit.is_none_or(|b| form.size.width_mm < b.size.width_mm)
        {
            best_fit = Some(form);
        }
    }
    best_fit.or(widest).cloned()
}

/// Select media without a user-specified paper name or scaling policy.
/// Exact forms and compatible rolls win; custom size is used only inside
/// driver-reported bounds; otherwise the smallest larger reported form is fit,
/// with a tolerant close-sheet fallback (e.g. A4 <-> Letter).
pub fn match_paper_auto(
    page: PageSize,
    capabilities: &PrinterCapabilities,
) -> Result<PaperMatch, PaperError> {
    if !page.width_mm.is_finite()
        || !page.height_mm.is_finite()
        || page.width_mm <= 0.0
        || page.height_mm <= 0.0
    {
        return Err(PaperError {
            page_width_mm: page.width_mm,
            page_height_mm: page.height_mm,
            policy: ScalePolicy::NearestLargerFit,
            available: capabilities
                .forms
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        });
    }

    // Exact sheet forms have priority.
    if let Some(form) = capabilities
        .forms
        .iter()
        .find(|form| valid_sheet(form) && same_size(form.size, page))
    {
        return Ok(PaperMatch {
            kind: MatchKind::Exact,
            form: form.clone(),
        });
    }

    // A roll can print a shorter/narrower raster without inventing another
    // paper size. Pick the narrowest roll that fits; retain the job's length.
    if let Some(form) = capabilities
        .forms
        .iter()
        .filter(|form| {
            form.roll
                && form.size.width_mm.is_finite()
                && form.size.width_mm > 0.0
                && page.width_mm <= form.size.width_mm + ROLL_WIDTH_TOLERANCE_MM
        })
        .min_by(|a, b| {
            a.size
                .width_mm
                .partial_cmp(&b.size.width_mm)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
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

    match match_paper(page, &capabilities.forms, ScalePolicy::NearestLargerFit) {
        Ok(matched) => Ok(matched),
        Err(err) => {
            // Tolerant close-sheet fallback (e.g. A4 <-> US Letter) so
            // slightly different regional sheet standards scale-to-fit instead
            // of failing the print job.
            let within_rel =
                |a: f64, b: f64| (a - b).abs() / b.max(1.0) <= CLOSE_SHEET_REL_TOLERANCE;
            let orient_delta = |f: &Form| -> Option<f64> {
                let direct = within_rel(f.size.width_mm, page.width_mm)
                    && within_rel(f.size.height_mm, page.height_mm);
                let rotated = within_rel(f.size.width_mm, page.height_mm)
                    && within_rel(f.size.height_mm, page.width_mm);
                match (direct, rotated) {
                    (true, _) => Some(
                        (f.size.width_mm - page.width_mm).abs()
                            + (f.size.height_mm - page.height_mm).abs(),
                    ),
                    (false, true) => Some(
                        (f.size.width_mm - page.height_mm).abs()
                            + (f.size.height_mm - page.width_mm).abs(),
                    ),
                    (false, false) => None,
                }
            };
            if let Some(form) = capabilities
                .forms
                .iter()
                .filter(|f| valid_sheet(f))
                .filter_map(|f| orient_delta(f).map(|d| (d, f)))
                .min_by(|(da, _), (db, _)| da.partial_cmp(db).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(_, f)| f.clone())
            {
                return Ok(PaperMatch {
                    kind: MatchKind::NearestLargerFit,
                    form,
                });
            }
            Err(err)
        }
    }
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
    fn sheet_form_for_pagination() {
        // A receipt strip (80 mm wide) on an office printer
        // paginates onto the office sheet (A4 class), not the
        // smallest listed form (A5).
        let picked = pick_sheet_form(&forms(), 80.0).unwrap();
        assert_eq!(picked.name, "A4");
        // Landscape-reported A4 form still matches office class.
        let landscape = vec![Form::sheet("A4 Landscape", PageSize::a4().rotated())];
        assert_eq!(
            pick_sheet_form(&landscape, 80.0).unwrap().name,
            "A4 Landscape"
        );
        // No office-class sheet (a small label printer in GDI
        // mode): the narrowest sheet that fits the width wins.
        let label_forms = vec![
            Form::sheet("100x150", PageSize::new(100.0, 150.0)),
            Form::sheet("60x40", PageSize::new(60.0, 40.0)),
        ];
        let picked = pick_sheet_form(&label_forms, 80.0).unwrap();
        assert_eq!(picked.name, "100x150");
        // Wider than every sheet: the widest sheet wins and the
        // content scales down to it.
        let picked = pick_sheet_form(&forms(), 300.0).unwrap();
        assert_eq!(picked.name, "Letter");
        // Roll-only media never paginates.
        let rolls = vec![Form {
            name: "80 x 3276 mm Roll".to_string(),
            size: PageSize::new(80.0, 3276.0),
            roll: true,
            paper_id: None,
        }];
        assert!(pick_sheet_form(&rolls, 80.0).is_none());
        assert!(pick_sheet_form(&[], 80.0).is_none());
        assert!(pick_sheet_form(&forms(), 0.0).is_none());
        assert!(pick_sheet_form(&forms(), -10.0).is_none());
        assert!(pick_sheet_form(&forms(), f64::NAN).is_none());
        assert!(pick_sheet_form(&forms(), f64::INFINITY).is_none());
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
        let bad_forms = vec![Form::sheet("Zero", PageSize::new(0.0, 0.0))];
        assert!(match_paper(PageSize::new(0.5, 0.5), &bad_forms, ScalePolicy::Strict).is_err());
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
        assert!(match_paper(
            PageSize::new(f64::NAN, 100.0),
            &forms(),
            ScalePolicy::CustomAllowed,
        )
        .is_err());
        assert!(match_paper(
            PageSize::new(0.0, 100.0),
            &forms(),
            ScalePolicy::CustomAllowed,
        )
        .is_err());
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
            provenance: Default::default(),
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
            provenance: Default::default(),
        };
        let selected = match_paper_auto(PageSize::new(72.0, 180.0), &caps).unwrap();
        assert_eq!(selected.kind, MatchKind::Roll);
        assert_eq!(selected.form.size.width_mm, 80.0);
        assert_eq!(selected.form.size.height_mm, 180.0);
        assert!(match_paper_auto(PageSize::new(90.0, 180.0), &caps).is_err());
    }

    #[test]
    fn narrowest_roll_wins_when_multiple_rolls_present() {
        let caps = PrinterCapabilities {
            forms: vec![Form::roll("80mm Roll", 80.0), Form::roll("58mm Roll", 58.0)],
            ..Default::default()
        };
        let selected = match_paper_auto(PageSize::new(57.5, 150.0), &caps).unwrap();
        assert_eq!(selected.kind, MatchKind::Roll);
        assert_eq!(selected.form.name, "58mm Roll");
    }

    #[test]
    fn automatic_media_uses_larger_form_when_custom_is_out_of_bounds() {
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
            provenance: Default::default(),
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
    fn a4_on_letter_only_and_letter_on_a4_only_succeed() {
        let letter_only = PrinterCapabilities {
            forms: vec![Form::sheet("Letter", PageSize::new(215.9, 279.4))],
            ..Default::default()
        };
        let m1 = match_paper_auto(PageSize::a4(), &letter_only).unwrap();
        assert_eq!(m1.kind, MatchKind::NearestLargerFit);
        assert_eq!(m1.form.name, "Letter");

        let a4_only = PrinterCapabilities {
            forms: vec![Form::sheet("A4", PageSize::a4())],
            ..Default::default()
        };
        let m2 = match_paper_auto(PageSize::new(215.9, 279.4), &a4_only).unwrap();
        assert_eq!(m2.kind, MatchKind::NearestLargerFit);
        assert_eq!(m2.form.name, "A4");
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

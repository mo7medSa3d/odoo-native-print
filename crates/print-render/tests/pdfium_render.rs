//! Integration tests for the PDFium renderer, run against the real
//! shared library (pinned build, docs/pdfium.md).
//!
//! Library location: $NATIVE_PRINT_PDFIUM_PATH, else
//! <repo>/tools/pdfium/ as installed by tools/fetch-pdfium.sh.
//! If the library is absent the tests skip with a warning — EXCEPT
//! when PDFIUM_TEST_REQUIRED=1 (CI sets it), where absence fails:
//! a green run that never rendered anything would be a false pass.

use std::path::PathBuf;

use print_core::PageSize;
use print_render::pdfium::{Pdfium, PdfiumError};
use print_render::{plan_render, RenderMode};

fn pdfium_or_skip() -> Option<std::sync::Arc<Pdfium>> {
    let required = std::env::var("PDFIUM_TEST_REQUIRED").is_ok();
    let repo_lib = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/pdfium")
        .canonicalize()
        .ok();
    let result = match &repo_lib {
        Some(dir) => Pdfium::load_from(Some(dir)),
        None => Pdfium::load(),
    };
    match result {
        Ok(p) => Some(p),
        Err(e) => {
            if required {
                panic!(
                    "PDFIUM_TEST_REQUIRED is set but PDFium did not load: {e}. \
                     Run tools/fetch-pdfium.sh first."
                );
            }
            eprintln!("skipping pdfium test, library unavailable: {e}");
            None
        }
    }
}

/// Minimal one-page PDF with a given MediaBox (points) and one filled
/// black rectangle, built with a correct xref table (same pattern as
/// the calibration generator).
fn minimal_pdf(width_pt: f64, height_pt: f64) -> Vec<u8> {
    let content = format!(
        "0 0 0 rg\n{:.2} {:.2} {:.2} {:.2} re f\n",
        width_pt * 0.25,
        height_pt * 0.25,
        width_pt * 0.5,
        height_pt * 0.5
    )
    .into_bytes();
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width_pt} {height_pt}] /Contents 4 0 R >>")
            .into_bytes(),
    ];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    offsets.push(out.len());
    out.extend_from_slice(b"4 0 obj\n");
    out.extend_from_slice(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
    out.extend_from_slice(&content);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let xref_pos = out.len();
    out.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer << /Size 5 /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF").as_bytes(),
    );
    out
}

#[test]
fn renders_calibration_a4_page_1_at_plan_dimensions() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    let bytes = print_render::calibration::calibration_pdf();
    let doc = pdfium.load_document(&bytes).expect("calibration PDF loads");
    assert_eq!(doc.page_count(), 1);
    let size = doc.page_size_mm(0).expect("page size");
    assert!((size.width_mm - 210.0).abs() < 1.5, "width {size:?}");
    assert!((size.height_mm - 297.0).abs() < 1.5, "height {size:?}");

    let plan = plan_render(PageSize::a4(), 150, RenderMode::Grayscale).expect("A4 plan");
    let page = doc
        .render_page(0, RenderMode::Grayscale, 150)
        .expect("render page 1");
    assert_eq!(page.width_px, plan.width_px);
    assert_eq!(page.height_px, plan.height_px);
    assert_eq!(page.dpi, plan.dpi);
    assert_eq!(
        page.pixels.len(),
        (plan.width_px as usize) * (plan.height_px as usize)
    );
    assert!(!page.is_blank(), "calibration page must not render blank");
    assert!(
        page.non_background_fraction() > 0.001,
        "rulers/text/square expected, got fraction {}",
        page.non_background_fraction()
    );
}

#[test]
fn renders_color_and_threshold_modes() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    let doc = pdfium
        .load_document(&minimal_pdf(595.28, 841.89))
        .expect("minimal PDF loads");
    let color = doc
        .render_page(0, RenderMode::Color, 100)
        .expect("color render");
    assert_eq!(
        color.pixels.len(),
        (color.width_px as usize) * (color.height_px as usize) * 4
    );
    // The filled rect covers the middle quarter of the area.
    assert!(color.non_background_fraction() > 0.1);

    let mono = doc
        .render_page(0, RenderMode::Threshold, 100)
        .expect("threshold render");
    let bpr = (mono.width_px as usize).div_ceil(8);
    assert_eq!(mono.pixels.len(), bpr * mono.height_px as usize);
    assert!(mono.non_background_fraction() > 0.1);
}

#[test]
fn huge_color_page_hits_the_memory_ceiling_loudly() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    // 1500 x 1500 mm at even 100 dpi in color needs ~139 MB > 128 MiB.
    let doc = pdfium
        .load_document(&minimal_pdf(4251.97, 4251.97))
        .expect("huge PDF loads");
    let err = doc
        .render_page(0, RenderMode::Color, 300)
        .expect_err("oversized render must be refused");
    assert!(
        matches!(err, PdfiumError::CeilingExceeded { .. }),
        "expected CeilingExceeded, got {err}"
    );
}

#[test]
fn garbage_bytes_are_rejected_loudly() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    let err = pdfium
        .load_document(b"this is not a pdf at all")
        .err()
        .expect("garbage must not load");
    assert!(
        matches!(err, PdfiumError::InvalidDocument { .. }),
        "expected InvalidDocument, got {err}"
    );
}

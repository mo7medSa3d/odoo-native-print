//! print-render: everything that turns input documents into printable
//! pages, platform-independently. PDFium (Windows phase) supplies the
//! authoritative rendering; this crate supplies PDF inspection for
//! queue-time planning, render planning under a memory ceiling, mixed-size
//! splitting, bitmap drawing, and the calibration page generator.

pub mod bitmap;
pub mod calibration;
pub mod image;
pub mod pdf;
pub mod pdfium;
pub mod plan;

pub use image::{
    decode_png, receipt_dots_for_paper, receipt_png_to_escpos, scale_to_width, ImageError,
};
pub use pdf::{parse_pdf_info, PdfInfo, PdfPageInfo, PdfParseError};
pub use pdfium::{PdfDocument, Pdfium, PdfiumError, RenderedPage};
pub use plan::{plan_render, split_subjobs, RenderMode, RenderPlan, SubJob};

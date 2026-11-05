//! print-win: Windows print backends (winspool + GDI).
//!
//! STATUS: the FFI surface in this crate is compile-verified only on
//! Windows CI (see .github/workflows/ci.yml, job `windows`). It cannot be
//! compiled or run on the Linux build machine, and no result from it is
//! claimed until the Windows job is green and the virtual-printer tests
//! (spike S5) pass there.
//!
//! What is implemented here:
//! - Default printer lookup (GetDefaultPrinterW).
//! - Paper forms for a printer (DeviceCapabilitiesW DC_PAPERS/DC_PAPERSIZE/DC_PAPERNAMES),
//!   returned as print-core Forms for the paper ladder (with roll paper detection
//!   and `dmPaperSize` IDs).
//! - RAW spooler submission (OpenPrinterW/StartDocW/StartPage/WritePrinter/
//!   EndPage/EndDoc/ClosePrinter) for ESC/POS and ZPL byte streams — this
//!   is the transport under the thermal/label modes.
//! - GDI bitmap backend (RC5+): PDF pages rendered by the shared PDFium
//!   renderer in print-render (or PNG/Canvas images), converted to DIBs by
//!   the platform-independent `dib` module (unit-tested everywhere), and
//!   blitted to the printer DC in `win::print_pdf_gdi` / `win::print_canvas_gdi`
//!   with `DocumentPropertiesW` `DEVMODEW` configuration (orientation, custom/matched
//!   paper size, input tray/bin via `DC_BINS`/`DC_BINNAMES`, duplex, color, and
//!   sub-job splitting for mixed-size documents).

pub mod dib;
pub mod discovery;

/// Options passed to the Windows GDI backend when creating the printer DC
/// (`DocumentPropertiesW` / `DEVMODEW` + `CreateDCW`) and blitting pages.
#[derive(Debug, Clone)]
pub struct GdiJobOptions {
    pub mode: print_render::RenderMode,
    pub dpi: u32,
    pub copies: u16,
    pub duplex: bool,
    pub color: bool,
    pub scale_policy: print_core::ScalePolicy,
    pub tray: Option<String>,
}

impl Default for GdiJobOptions {
    fn default() -> Self {
        Self {
            mode: print_render::RenderMode::Color,
            dpi: 300,
            copies: 1,
            duplex: false,
            color: true,
            scale_policy: print_core::ScalePolicy::CustomAllowed,
            tray: None,
        }
    }
}

/// Failure of a GDI print entry point (the `*_detailed` functions).
/// `ambiguous` is true when pages may already have reached paper
/// before the failure — ADR-0004's cardinal rule: an ambiguous job
/// must never be auto-reprinted, because the "failed" copy may be
/// sitting in the output tray. Setup failures (PDFium load, paper
/// matching, CreateDC, StartDoc before any page) carry
/// `ambiguous: false`: nothing was printed, so a retry is safe.
/// The plain `print_*_gdi*` wrappers collapse this to the message
/// string for callers that do not classify errors (audit M5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GdiPrintError {
    pub message: String,
    pub ambiguous: bool,
}

impl GdiPrintError {
    /// A failure before anything could reach paper.
    pub fn setup(message: String) -> Self {
        Self {
            message,
            ambiguous: false,
        }
    }
    /// A failure after printing started: output may exist.
    pub fn mid_print(message: String) -> Self {
        Self {
            message,
            ambiguous: true,
        }
    }
}

impl std::fmt::Display for GdiPrintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for GdiPrintError {}

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{
    default_printer_name, discover_printers, paper_forms, print_canvas_gdi,
    print_canvas_gdi_configured, print_canvas_gdi_detailed, print_pdf_gdi, print_pdf_gdi_copies,
    print_pdf_gdi_detailed, print_pdf_gdi_with_options, print_raw, print_raw_phased,
    printer_capabilities, printer_health, printer_status, RawSendError,
};

#[cfg(not(windows))]
mod stub {
    use super::GdiJobOptions;
    use print_core::Form;
    use thiserror::Error;
    #[derive(Debug, Error)]
    #[error("print-win is only available on Windows")]
    pub struct Unsupported;
    pub fn default_printer_name() -> Result<String, Unsupported> {
        Err(Unsupported)
    }
    pub fn paper_forms(_printer: &str) -> Result<Vec<Form>, Unsupported> {
        Err(Unsupported)
    }
    pub fn printer_capabilities(
        _printer: &str,
    ) -> Result<print_core::PrinterCapabilities, Unsupported> {
        Err(Unsupported)
    }
    pub fn print_raw(_printer: &str, _title: &str, _bytes: &[u8]) -> Result<u32, Unsupported> {
        Err(Unsupported)
    }
    /// Mirror of the Windows `win::RawSendError` so callers can
    /// match on the failure phase on every platform.
    #[derive(Debug)]
    pub struct RawSendError {
        pub pre_write: bool,
        pub message: String,
    }
    impl std::fmt::Display for RawSendError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }
    impl std::error::Error for RawSendError {}
    pub fn print_raw_phased(
        _printer: &str,
        _title: &str,
        _bytes: &[u8],
    ) -> Result<u32, RawSendError> {
        Err(RawSendError {
            pre_write: true,
            message: "print-win is only available on Windows".to_string(),
        })
    }
    pub fn print_pdf_gdi(
        _printer: &str,
        _title: &str,
        _pdf_bytes: &[u8],
        _mode: print_render::RenderMode,
        _dpi: u32,
    ) -> Result<usize, Unsupported> {
        Err(Unsupported)
    }
    pub fn print_pdf_gdi_copies(
        _printer: &str,
        _title: &str,
        _pdf_bytes: &[u8],
        _mode: print_render::RenderMode,
        _dpi: u32,
        _copies: u16,
    ) -> Result<usize, Unsupported> {
        Err(Unsupported)
    }
    pub fn print_pdf_gdi_with_options(
        _printer: &str,
        _title: &str,
        _pdf_bytes: &[u8],
        _opts: &GdiJobOptions,
    ) -> Result<usize, Unsupported> {
        Err(Unsupported)
    }
    pub fn print_pdf_gdi_detailed(
        _printer: &str,
        _title: &str,
        _pdf_bytes: &[u8],
        _opts: &GdiJobOptions,
    ) -> Result<usize, super::GdiPrintError> {
        Err(super::GdiPrintError::setup(Unsupported.to_string()))
    }
    pub fn print_canvas_gdi(
        _printer: &str,
        _title: &str,
        _canvas: &print_render::bitmap::Canvas,
        _copies: u16,
    ) -> Result<usize, Unsupported> {
        Err(Unsupported)
    }
    pub fn print_canvas_gdi_detailed(
        _printer: &str,
        _title: &str,
        _canvas: &print_render::bitmap::Canvas,
        _copies: u16,
    ) -> Result<usize, super::GdiPrintError> {
        Err(super::GdiPrintError::setup(Unsupported.to_string()))
    }
    pub fn print_canvas_gdi_configured(
        _printer: &str,
        _title: &str,
        _canvas: &print_render::bitmap::Canvas,
        _copies: u16,
        _tray: Option<&str>,
        _roll_width_mm: Option<f64>,
    ) -> Result<usize, super::GdiPrintError> {
        Err(super::GdiPrintError::setup(Unsupported.to_string()))
    }
    pub fn discover_printers() -> Result<Vec<print_core::PrinterInfo>, Unsupported> {
        Err(Unsupported)
    }
    pub fn printer_status(_printer: &str) -> Result<print_core::PrinterStatus, Unsupported> {
        Err(Unsupported)
    }
    pub fn printer_health(_printer: &str) -> Result<print_core::PrinterHealth, Unsupported> {
        Err(Unsupported)
    }
}

#[cfg(not(windows))]
pub use stub::{
    default_printer_name, discover_printers, paper_forms, print_canvas_gdi,
    print_canvas_gdi_configured, print_canvas_gdi_detailed, print_pdf_gdi, print_pdf_gdi_copies,
    print_pdf_gdi_detailed, print_pdf_gdi_with_options, print_raw, print_raw_phased,
    printer_capabilities, printer_health, printer_status, RawSendError, Unsupported,
};

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
//! - Paper forms for a printer (DeviceCapabilitiesW DC_PAPERS/DC_PAPERSIZE),
//!   returned as print-core Forms for the paper ladder.
//! - RAW spooler submission (OpenPrinterW/StartDocW/StartPage/WritePrinter/
//!   EndPage/EndDoc/ClosePrinter) for ESC/POS and ZPL byte streams — this
//!   is the transport under the thermal/label modes.
//!
//! GDI bitmap backend (RC5): PDF pages rendered by the shared PDFium
//! renderer in print-render, converted to DIBs by the platform-
//! independent `dib` module (unit-tested everywhere), and blitted to
//! the printer DC in `win::print_pdf_gdi`. Code-complete; its first
//! compile is the Windows CI job, and field proof is spike S5.

pub mod dib;
pub mod discovery;

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{
    default_printer_name, discover_printers, paper_forms, print_pdf_gdi, print_raw, printer_status,
};

#[cfg(not(windows))]
mod stub {
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
    pub fn print_raw(_printer: &str, _title: &str, _bytes: &[u8]) -> Result<u32, Unsupported> {
        Err(Unsupported)
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
    pub fn discover_printers() -> Result<Vec<print_core::PrinterInfo>, Unsupported> {
        Err(Unsupported)
    }
    pub fn printer_status(_printer: &str) -> Result<print_core::PrinterStatus, Unsupported> {
        Err(Unsupported)
    }
}

#[cfg(not(windows))]
pub use stub::{
    default_printer_name, discover_printers, paper_forms, print_pdf_gdi, print_raw, printer_status,
    Unsupported,
};

//! print-raw: raw printer languages and transports.
//! ESC/POS raster is the primary path for thermal/Arabic (always raster for
//! RTL, per spec). ZPL covers label bitmaps. Transports (spooler RAW,
//! TCP 9100) are platform/network code built on top of these encoders.

pub mod epl;
pub mod escpos;
pub mod mono;
pub mod tcp9100;
pub mod tspl;
pub mod zpl;

pub use escpos::EscPos;
pub use mono::{MonoBitmap, MonoError};
pub use zpl::zpl_graphic_field;

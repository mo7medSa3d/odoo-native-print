//! Win32 FFI for winspool.drv and gdi32.dll. Only simple, stable buffer APIs are used.
//! All wide strings are NUL-terminated UTF-16; every handle opened here is
//! closed on every path out of its function.

use crate::{GdiJobOptions, GdiPrintError, RawSendError};
use print_core::paper::{
    match_paper, match_paper_auto, pick_sheet_form, Form, MatchKind, PaperMatch, ScalePolicy,
};
use print_core::PageSize;
use print_render::split_subjobs;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;

type Handle = *mut c_void;
type Bool = i32;

struct PrinterHandle(Handle);

impl Drop for PrinterHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ClosePrinter(self.0) };
        }
    }
}

struct PrinterDc(Hdc);

impl Drop for PrinterDc {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DeleteDC(self.0) };
        }
    }
}

#[repr(C)]
struct DocInfo1W {
    doc_name: *const u16,
    output_file: *const u16,
    datatype: *const u16,
}

#[link(name = "winspool")]
extern "system" {
    fn GetDefaultPrinterW(name: *mut u16, size: *mut u32) -> Bool;
    fn OpenPrinterW(name: *const u16, handle: *mut Handle, defaults: *mut c_void) -> Bool;
    fn ClosePrinter(handle: Handle) -> Bool;
    fn DocumentPropertiesW(
        hwnd: Handle,
        printer: Handle,
        device_name: *const u16,
        devmode_output: *mut u8,
        devmode_input: *const u8,
        mode: u32,
    ) -> i32;
    fn DeviceCapabilitiesW(
        device: *const u16,
        port: *const u16,
        capability: u16,
        output: *mut c_void,
        devmode: *const c_void,
    ) -> i32;
    fn StartDocPrinterW(handle: Handle, level: u32, doc_info: *const DocInfo1W) -> u32;
    fn StartPagePrinter(handle: Handle) -> Bool;
    fn WritePrinter(handle: Handle, buffer: *const c_void, count: u32, written: *mut u32) -> Bool;
    fn EndPagePrinter(handle: Handle) -> Bool;
    fn EndDocPrinter(handle: Handle) -> Bool;
    fn AbortPrinter(handle: Handle) -> Bool;
}

const DC_PAPERS: u16 = 2;
const DC_PAPERSIZE: u16 = 3;
const DC_MINEXTENT: u16 = 4;
const DC_MAXEXTENT: u16 = 5;
const DC_BINS: u16 = 6;
const DC_DUPLEX: u16 = 7;
const DC_BINNAMES: u16 = 12;
const DC_ENUMRESOLUTIONS: u16 = 13;
const DC_PAPERNAMES: u16 = 16;
const DC_COPIES: u16 = 18;
const DC_COLORDEVICE: u16 = 32;

// DocumentPropertiesW flags & DEVMODEW constants (wingdi.h / winspool.h).
const DM_OUT_BUFFER: u32 = 2;
const DM_IN_BUFFER: u32 = 8;

const DM_ORIENTATION: u32 = 0x0000_0001;
const DM_PAPERSIZE: u32 = 0x0000_0002;
const DM_PAPERLENGTH: u32 = 0x0000_0004;
const DM_PAPERWIDTH: u32 = 0x0000_0008;
const DM_DEFAULTSOURCE: u32 = 0x0000_0200;
const DM_COLOR: u32 = 0x0000_0800;
const DM_DUPLEX: u32 = 0x0000_1000;
const DM_FORMNAME: u32 = 0x0001_0000;

const DMORIENT_PORTRAIT: i16 = 1;
const DMORIENT_LANDSCAPE: i16 = 2;
const DMPAPER_USER: i16 = 256;
const DMCOLOR_MONOCHROME: i16 = 1;
const DMCOLOR_COLOR: i16 = 2;
const DMDUP_SIMPLEX: i16 = 1;
const DMDUP_VERTICAL: i16 = 2;

/// Public prefix of Win32 `DEVMODEW` (wingdi.h) up through `dmFormName` (168 bytes).
/// Driver-private bytes follow `dmSize` in the buffer allocated from
/// `DocumentPropertiesW(..., 0)`.
#[repr(C)]
struct DevModePrefixW {
    dm_device_name: [u16; 32],
    dm_spec_version: u16,
    dm_driver_version: u16,
    dm_size: u16,
    dm_driver_extra: u16,
    dm_fields: u32,
    dm_orientation: i16,
    dm_paper_size: i16,
    dm_paper_length: i16,
    dm_paper_width: i16,
    dm_scale: i16,
    dm_copies: i16,
    dm_default_source: i16,
    dm_print_quality: i16,
    dm_color: i16,
    dm_duplex: i16,
    dm_y_resolution: i16,
    dm_tt_option: i16,
    dm_collate: i16,
    dm_form_name: [u16; 32],
}

// --- GDI (gdi32) for the PDF bitmap backend ---------------------------

type Hdc = *mut c_void;

#[repr(C)]
struct DocInfoW {
    size: i32,
    doc_name: *const u16,
    output: *const u16,
    datatype: *const u16,
    fw_type: u32,
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateDCW(
        driver: *const u16,
        device: *const u16,
        output: *const u16,
        init_data: *const c_void,
    ) -> Hdc;
    fn DeleteDC(dc: Hdc) -> Bool;
    fn GetDeviceCaps(dc: Hdc, index: i32) -> i32;
    fn StartDocW(dc: Hdc, doc_info: *const DocInfoW) -> i32;
    fn StartPage(dc: Hdc) -> i32;
    fn EndPage(dc: Hdc) -> i32;
    fn EndDoc(dc: Hdc) -> i32;
    fn AbortDoc(dc: Hdc) -> i32;
    fn StretchDIBits(
        dc: Hdc,
        x_dest: i32,
        y_dest: i32,
        dest_w: i32,
        dest_h: i32,
        x_src: i32,
        y_src: i32,
        src_w: i32,
        src_h: i32,
        bits: *const c_void,
        bitmap_info: *const c_void,
        usage: u32,
        rop: u32,
    ) -> i32;
}

const HORZRES: i32 = 8;
const VERTRES: i32 = 10;
const LOGPIXELSX: i32 = 88;
const LOGPIXELSY: i32 = 90;
const DIB_RGB_COLORS: u32 = 0;
const SRCCOPY: u32 = 0x00CC_0020;
const GDI_ERROR: i32 = -1;

/// Resolve a human-readable tray/bin name (e.g. "Tray 1", "Auto") or numeric ID
/// to a driver bin code (`dmDefaultSource`) via `DeviceCapabilitiesW(DC_BINS / DC_BINNAMES)`.
fn resolve_printer_bin_id(printer_wide: &[u16], tray_spec: &str) -> Option<i16> {
    let trimmed = tray_spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(num) = trimmed.parse::<i16>() {
        return Some(num);
    }
    let count = unsafe {
        DeviceCapabilitiesW(
            printer_wide.as_ptr(),
            std::ptr::null(),
            DC_BINS,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if !(1..=4096).contains(&count) {
        return None;
    }
    let name_count = unsafe {
        DeviceCapabilitiesW(
            printer_wide.as_ptr(),
            std::ptr::null(),
            DC_BINNAMES,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if !(1..=4096).contains(&name_count) {
        return None;
    }
    let n_ids = count as usize;
    let n_names = name_count as usize;
    let mut bin_ids = vec![0u16; n_ids];
    let got_ids = unsafe {
        DeviceCapabilitiesW(
            printer_wide.as_ptr(),
            std::ptr::null(),
            DC_BINS,
            bin_ids.as_mut_ptr() as *mut c_void,
            std::ptr::null(),
        )
    };
    // DC_BINNAMES: each entry is a 24-WCHAR block (Microsoft Learn).
    let mut bin_names = vec![0u16; n_names * 24];
    let got_names = unsafe {
        DeviceCapabilitiesW(
            printer_wide.as_ptr(),
            std::ptr::null(),
            DC_BINNAMES,
            bin_names.as_mut_ptr() as *mut c_void,
            std::ptr::null(),
        )
    };
    if got_ids <= 0 || got_names <= 0 {
        return None;
    }
    let count_ok = (got_ids as usize)
        .min(got_names as usize)
        .min(n_ids)
        .min(n_names);
    let target = trimmed.to_ascii_lowercase();
    let names: Vec<String> = (0..count_ok)
        .map(|i| {
            let block = &bin_names[i * 24..(i + 1) * 24];
            let end = block.iter().position(|&c| c == 0).unwrap_or(24);
            String::from_utf16_lossy(&block[..end])
                .trim()
                .to_ascii_lowercase()
        })
        .collect();
    // Exact match first: a substring pass would resolve "Tray 1" to
    // "Tray 10" whenever the driver enumerates it first. (Audit L7.)
    for (i, name) in names.iter().enumerate() {
        if !name.is_empty() && *name == target {
            return Some(bin_ids[i] as i16);
        }
    }
    for (i, name) in names.iter().enumerate() {
        if !name.is_empty() && name.contains(&target) {
            return Some(bin_ids[i] as i16);
        }
    }
    None
}

/// Build a validated `DEVMODEW` buffer for `printer` configured for `page_size`,
/// `matched_form`, `duplex`, `color`, and optional `tray` using the 3-step
/// Microsoft Learn `DocumentPropertiesW` pattern. Returns `None` if the driver
/// does not expose a modifiable `DEVMODEW` (caller falls back to default `CreateDCW`).
fn aligned_byte_buf(bytes: usize) -> Vec<u64> {
    vec![0u64; bytes.div_ceil(std::mem::size_of::<u64>())]
}

fn build_printer_devmode(
    printer: &str,
    page_size: PageSize,
    matched_form: Option<(&Form, MatchKind)>,
    duplex: Option<bool>,
    color: bool,
    tray: Option<&str>,
) -> Option<Vec<u64>> {
    let name = wide(printer);
    let mut raw_handle: Handle = std::ptr::null_mut();
    let ok = unsafe { OpenPrinterW(name.as_ptr(), &mut raw_handle, std::ptr::null_mut()) };
    if ok == 0 || raw_handle.is_null() {
        return None;
    }
    let handle = PrinterHandle(raw_handle);
    unsafe {
        let needed = DocumentPropertiesW(
            std::ptr::null_mut(),
            handle.0,
            name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null(),
            0,
        );
        if needed < std::mem::size_of::<DevModePrefixW>() as i32 {
            None
        } else {
            let mut buf = aligned_byte_buf(needed as usize);
            let got = DocumentPropertiesW(
                std::ptr::null_mut(),
                handle.0,
                name.as_ptr(),
                buf.as_mut_ptr() as *mut u8,
                std::ptr::null(),
                DM_OUT_BUFFER,
            );
            if got < 0 {
                None
            } else {
                let dm = &mut *(buf.as_mut_ptr() as *mut DevModePrefixW);
                let is_roll = matches!(matched_form, Some((_, MatchKind::Roll)));
                // Orientation: roll printers feed continuous media lengthwise
                // and cannot rotate 90 degrees even for short receipts (e.g. 80x50mm).
                dm.dm_orientation = if !is_roll && page_size.is_landscape() {
                    DMORIENT_LANDSCAPE
                } else {
                    DMORIENT_PORTRAIT
                };
                dm.dm_fields |= DM_ORIENTATION;

                // Paper size / form selection (sets both dmPaperSize from DC_PAPERS
                // when known and dmFormName for Named Forms).
                let unrotated_w = page_size.width_mm.min(page_size.height_mm);
                let unrotated_h = page_size.width_mm.max(page_size.height_mm);
                match matched_form {
                    Some((form, MatchKind::Roll)) => {
                        if let Some(pid) = form.paper_id {
                            dm.dm_paper_size = pid as i16;
                            dm.dm_fields =
                                (dm.dm_fields & !(DM_PAPERWIDTH | DM_PAPERLENGTH)) | DM_PAPERSIZE;
                        } else {
                            dm.dm_paper_size = DMPAPER_USER;
                            dm.dm_paper_width =
                                (form.size.width_mm * 10.0).round().clamp(100.0, 32000.0) as i16;
                            dm.dm_paper_length =
                                (page_size.height_mm * 10.0).round().clamp(100.0, 32000.0) as i16;
                            dm.dm_fields |= DM_PAPERSIZE | DM_PAPERWIDTH | DM_PAPERLENGTH;
                        }
                        if !form.name.is_empty() && form.name != "Custom" {
                            dm.dm_form_name = [0u16; 32];
                            for (idx, ch) in form.name.encode_utf16().take(31).enumerate() {
                                dm.dm_form_name[idx] = ch;
                            }
                            dm.dm_fields |= DM_FORMNAME;
                        }
                    }
                    Some((
                        form,
                        MatchKind::Exact
                        | MatchKind::NearestLargerCentered
                        | MatchKind::NearestLargerFit,
                    )) if !form.name.is_empty() && form.name != "Custom" => {
                        dm.dm_fields &= !(DM_PAPERWIDTH | DM_PAPERLENGTH);
                        if let Some(pid) = form.paper_id {
                            dm.dm_paper_size = pid as i16;
                            dm.dm_fields |= DM_PAPERSIZE;
                        }
                        dm.dm_form_name = [0u16; 32];
                        for (idx, ch) in form.name.encode_utf16().take(31).enumerate() {
                            dm.dm_form_name[idx] = ch;
                        }
                        dm.dm_fields |= DM_FORMNAME;
                    }
                    _ => {
                        dm.dm_paper_size = DMPAPER_USER;
                        dm.dm_paper_width =
                            (unrotated_w * 10.0).round().clamp(100.0, 32000.0) as i16;
                        dm.dm_paper_length =
                            (unrotated_h * 10.0).round().clamp(100.0, 32000.0) as i16;
                        dm.dm_fields = (dm.dm_fields & !DM_FORMNAME)
                            | DM_PAPERSIZE
                            | DM_PAPERWIDTH
                            | DM_PAPERLENGTH;
                    }
                }

                // Input tray / bin selection
                if let Some(t) = tray {
                    if let Some(bin_id) = resolve_printer_bin_id(&name, t) {
                        dm.dm_default_source = bin_id;
                        dm.dm_fields |= DM_DEFAULTSOURCE;
                    }
                }

                // Duplex & Color. Duplex is tri-state: an
                // unspecified duplex leaves the DEVMODE field
                // untouched so the DRIVER DEFAULT applies — only
                // an explicit choice (the user's, or a saved
                // per-printer default) may turn a job double-sided
                // or force it simplex (external review, 2026-11-07).
                if let Some(d) = duplex {
                    dm.dm_duplex = if d { DMDUP_VERTICAL } else { DMDUP_SIMPLEX };
                    dm.dm_fields |= DM_DUPLEX;
                }
                dm.dm_color = if color {
                    DMCOLOR_COLOR
                } else {
                    DMCOLOR_MONOCHROME
                };
                dm.dm_fields |= DM_COLOR;

                let merged = DocumentPropertiesW(
                    std::ptr::null_mut(),
                    handle.0,
                    name.as_ptr(),
                    buf.as_mut_ptr() as *mut u8,
                    buf.as_ptr() as *const u8,
                    DM_IN_BUFFER | DM_OUT_BUFFER,
                );
                if merged < 0 {
                    None
                } else {
                    Some(buf)
                }
            }
        }
    }
}

/// Print a PDF through GDI: each page is rendered to a bitmap by the
/// shared PDFium renderer (print-render) and blitted to the printer
/// device context. Returns the number of pages sent.
pub fn print_pdf_gdi(
    printer: &str,
    title: &str,
    pdf_bytes: &[u8],
    mode: print_render::RenderMode,
    dpi: u32,
) -> Result<usize, String> {
    print_pdf_gdi_copies(printer, title, pdf_bytes, mode, dpi, 1)
}

/// Print a PDF through GDI with an explicit copy count (`1..=99`).
pub fn print_pdf_gdi_copies(
    printer: &str,
    title: &str,
    pdf_bytes: &[u8],
    mode: print_render::RenderMode,
    dpi: u32,
    copies: u16,
) -> Result<usize, String> {
    let opts = GdiJobOptions {
        mode,
        dpi,
        copies: copies.max(1),
        duplex: None,
        color: mode == print_render::RenderMode::Color,
        scale_policy: ScalePolicy::CustomAllowed,
        tray: None,
    };
    print_pdf_gdi_with_options(printer, title, pdf_bytes, &opts)
}

/// Print a PDF through GDI with full `GdiJobOptions`:
/// - Splits mixed-size PDFs into consecutive sub-jobs (`split_subjobs`)
/// - Matches paper per sub-job (`match_paper`)
/// - Configures `DEVMODEW` (orientation, paper size/form, tray/bin, duplex, color) via `DocumentPropertiesW`
/// - Honours `MatchKind::NearestLargerCentered` (100% scale centred) vs `Fit`/`Exact`.
pub fn print_pdf_gdi_with_options(
    printer: &str,
    title: &str,
    pdf_bytes: &[u8],
    opts: &GdiJobOptions,
) -> Result<usize, String> {
    print_pdf_gdi_detailed(printer, title, pdf_bytes, opts).map_err(|e| e.message)
}

/// Like `print_pdf_gdi_with_options`, but the error says whether any
/// output may already have reached paper (`GdiPrintError::ambiguous`,
/// audit M5): setup failures happen before the first page and are
/// safe to retry; failures after printing started are not.
pub fn print_pdf_gdi_detailed(
    printer: &str,
    title: &str,
    pdf_bytes: &[u8],
    opts: &GdiJobOptions,
) -> Result<usize, GdiPrintError> {
    let pdfium = print_render::Pdfium::load().map_err(|e| GdiPrintError::setup(e.to_string()))?;
    let doc = pdfium
        .load_document(pdf_bytes)
        .map_err(|e| GdiPrintError::setup(e.to_string()))?;
    let pages = doc.page_count();
    if pages == 0 {
        return Err(GdiPrintError::setup("PDF has no pages".to_string()));
    }
    let mut page_sizes = Vec::with_capacity(pages);
    for i in 0..pages {
        page_sizes.push(
            doc.page_size_mm(i)
                .map_err(|e| GdiPrintError::setup(e.to_string()))?,
        );
    }
    let subjobs = split_subjobs(&page_sizes);
    let capabilities = match printer_capabilities(printer) {
        Ok(c) => Some(c),
        Err(e) => {
            if opts.scale_policy == ScalePolicy::Strict {
                return Err(GdiPrintError::setup(e));
            }
            eprintln!("Yaseir Print: capabilities unreadable for {printer:?} ({e}); falling back to driver defaults");
            None
        }
    };
    let name = wide(printer);
    let mut total_sent = 0usize;

    for _copy in 0..opts.copies.max(1) {
        for sub in &subjobs {
            // A failure inside the copies loop is a setup failure
            // only when no earlier copy/sub-job reached paper yet.
            let stage_err = |message: String, sent: usize| GdiPrintError {
                message,
                ambiguous: sent > 0,
            };
            let matched = match &capabilities {
                None => None,
                Some(caps) if caps.forms.is_empty() && !caps.custom_size_supported => None,
                Some(caps) if opts.scale_policy == ScalePolicy::CustomAllowed => {
                    match match_paper_auto(sub.size, caps) {
                        Ok(m) => Some(m),
                        Err(e) => {
                            eprintln!("Yaseir Print: paper match fallback for {printer:?} ({e})");
                            None
                        }
                    }
                }
                Some(caps) => match match_paper(sub.size, &caps.forms, opts.scale_policy) {
                    Ok(m) => Some(m),
                    Err(e) => {
                        if opts.scale_policy == ScalePolicy::Strict {
                            return Err(stage_err(e.to_string(), total_sent));
                        }
                        eprintln!("Yaseir Print: paper match fallback for {printer:?} ({e})");
                        None
                    }
                },
            };
            let match_ref = matched.as_ref().map(|m| (&m.form, m.kind));
            let mut match_kind = matched
                .as_ref()
                .map(|m| m.kind)
                .unwrap_or(MatchKind::NearestLargerFit);
            let devmode = build_printer_devmode(
                printer,
                sub.size,
                match_ref,
                opts.duplex,
                opts.color,
                opts.tray.as_deref(),
            );
            let init_ptr = devmode
                .as_ref()
                .map(|b| b.as_ptr() as *const c_void)
                .unwrap_or(std::ptr::null());
            let mut dc =
                unsafe { CreateDCW(std::ptr::null(), name.as_ptr(), std::ptr::null(), init_ptr) };
            if dc.is_null() && !init_ptr.is_null() {
                eprintln!(
                    "Yaseir Print: CreateDC with custom DEVMODE failed for {printer:?} ({}); retrying with driver defaults",
                    last_error()
                );
                dc = unsafe {
                    CreateDCW(
                        std::ptr::null(),
                        name.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                    )
                };
                match_kind = MatchKind::NearestLargerFit;
            }
            if dc.is_null() {
                return Err(stage_err(
                    format!("CreateDC for {printer:?}: {}", last_error()),
                    total_sent,
                ));
            }
            let dc = PrinterDc(dc);
            let sub_result = print_subjob_gdi(
                dc.0,
                &doc,
                title,
                sub.start_page,
                sub.page_count,
                sub.size,
                match_kind,
                opts.mode,
                opts.dpi,
            );
            match sub_result {
                Ok(sent) => total_sent += sent,
                Err(mut e) => {
                    // Pages from earlier copies/sub-jobs are already
                    // out: the failure is ambiguous even when this
                    // sub-job itself failed before its first page.
                    if total_sent > 0 {
                        e.ambiguous = true;
                    }
                    return Err(e);
                }
            }
        }
    }
    Ok(total_sent)
}

/// Print a decoded grayscale `Canvas` image (e.g. a PNG receipt routed
/// to an office GDI profile) via GDI `StretchDIBits`.
pub fn print_canvas_gdi(
    printer: &str,
    title: &str,
    canvas: &print_render::bitmap::Canvas,
    copies: u16,
) -> Result<usize, String> {
    print_canvas_gdi_detailed(printer, title, canvas, copies).map_err(|e| e.message)
}

/// Like `print_canvas_gdi`, but the error carries the ADR-0004
/// ambiguity flag (audit M5): failures before StartDoc are safe to
/// retry; a blit failure after the document started is not.
pub fn print_canvas_gdi_detailed(
    printer: &str,
    title: &str,
    canvas: &print_render::bitmap::Canvas,
    copies: u16,
) -> Result<usize, GdiPrintError> {
    print_canvas_gdi_configured(printer, title, canvas, copies, None, None)
}

/// `print_canvas_gdi_detailed` plus the job's tray selection and
/// the receipt roll width reported with the job (owner brief,
/// 2026-11-05): when the roll width is known it defines the page
/// width exactly (the raster's own DPI is only a fallback), so a
/// POS receipt lands on paper of its real width — a matching roll
/// form, a custom size, or the closest larger form.
pub fn print_canvas_gdi_configured(
    printer: &str,
    title: &str,
    canvas: &print_render::bitmap::Canvas,
    copies: u16,
    tray: Option<&str>,
    roll_width_mm: Option<f64>,
) -> Result<usize, GdiPrintError> {
    if canvas.width == 0 || canvas.height == 0 {
        return Err(GdiPrintError::setup("empty image canvas".to_string()));
    }
    match print_canvas_gdi_with_caps(printer, title, canvas, copies, tray, roll_width_mm) {
        Ok(pages) => Ok(pages),
        Err(error) if !error.ambiguous => {
            // Setup-stage failure only: capabilities unreadable, no
            // forms reported, the chosen form not applicable, the
            // DC refused — the signature of quirky virtual writers
            // (OneNote / PDF printers) whose drivers reject forced
            // forms but print perfectly well at their own defaults.
            // Nothing reached paper, so printing the same image on
            // the driver's default media cannot duplicate it
            // (ADR-0004-safe): the image is scaled to the printable
            // area and paginated, exactly like a browser printing a
            // long receipt (owner field report, 2026-11-06: a POS
            // receipt to a virtual printer died in setup).
            eprintln!(
                "Yaseir Print: caps-based canvas print to {printer:?} failed in setup ({}); retrying on the driver's default media",
                error.message
            );
            print_canvas_gdi_default_media(printer, title, canvas, copies, roll_width_mm)
        }
        Err(error) => Err(error),
    }
}

/// Print a canvas on the driver's OWN default media: no forced
/// form, no custom devmode — CreateDC with a null DEVMODE, then
/// the shared sheet pagination over the DC's printable area. The
/// last-resort path for drivers that cannot report or apply forms.
fn print_canvas_gdi_default_media(
    printer: &str,
    title: &str,
    canvas: &print_render::bitmap::Canvas,
    copies: u16,
    roll_width_mm: Option<f64>,
) -> Result<usize, GdiPrintError> {
    let name = wide(printer);
    let dc = unsafe {
        CreateDCW(
            std::ptr::null(),
            name.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if dc.is_null() {
        return Err(GdiPrintError::setup(format!(
            "CreateDC for {printer:?}: {}",
            last_error()
        )));
    }
    let dc = PrinterDc(dc);
    let doc_name = wide(title);
    let info = DocInfoW {
        size: std::mem::size_of::<DocInfoW>() as i32,
        doc_name: doc_name.as_ptr(),
        output: std::ptr::null(),
        datatype: std::ptr::null(),
        fw_type: 0,
    };
    if unsafe { StartDocW(dc.0, &info) } <= 0 {
        return Err(GdiPrintError::setup(format!("StartDoc: {}", last_error())));
    }
    match print_canvas_pages(dc.0, canvas, copies.max(1), roll_width_mm) {
        Ok(pages) => {
            if unsafe { EndDoc(dc.0) } <= 0 {
                return Err(GdiPrintError::mid_print(format!(
                    "EndDoc: {}",
                    last_error()
                )));
            }
            Ok(pages)
        }
        Err(error) => {
            // The document started; pages may already be written —
            // ambiguous per ADR-0004, like the caps path.
            unsafe { AbortDoc(dc.0) };
            Err(GdiPrintError::mid_print(error))
        }
    }
}

fn print_canvas_gdi_with_caps(
    printer: &str,
    title: &str,
    canvas: &print_render::bitmap::Canvas,
    copies: u16,
    tray: Option<&str>,
    roll_width_mm: Option<f64>,
) -> Result<usize, GdiPrintError> {
    // POS/kitchen PNG jobs that use GDI must obey the same media contract as
    // PDF jobs: derive the physical page geometry from the 203-DPI raster,
    // then select an exact/roll/custom/larger media form from the driver.
    // A strip taller than every sheet is not a mismatch: it paginates
    // across sheets (see the fallback below).
    let page_size = match roll_width_mm {
        Some(w) if (20.0..=500.0).contains(&w) => {
            PageSize::new(w, w * canvas.height as f64 / canvas.width as f64)
        }
        _ => PageSize::new(
            canvas.width as f64 * 25.4 / 203.0,
            canvas.height as f64 * 25.4 / 203.0,
        ),
    };
    let capabilities =
        printer_capabilities(printer).map_err(|e| GdiPrintError::setup(e.to_string()))?;
    if capabilities.forms.is_empty() && !capabilities.custom_size_supported {
        return Err(GdiPrintError::setup(
            "PRINTER_MEDIA_UNAVAILABLE: Windows driver reported no supported forms or custom-size bounds"
                .to_string(),
        ));
    }
    let matched = match match_paper_auto(page_size, &capabilities) {
        Ok(m) => m,
        Err(error) => match pick_sheet_form(&capabilities.forms, page_size.width_mm) {
            // The strip fits no form whole: it paginates onto the
            // best sheet below instead of refusing the job — a
            // receipt's height is not a paper mismatch (owner
            // directive, 2026-11-05: POS prints to any printer).
            Some(form) => PaperMatch {
                kind: MatchKind::NearestLargerFit,
                form,
            },
            None => {
                return Err(GdiPrintError::setup(format!(
                    "PAPER_MISMATCH: no safe media form for the receipt image: {error}"
                )));
            }
        },
    };
    let media_page_size = matched.form.size;
    let devmode = build_printer_devmode(
        printer,
        media_page_size,
        Some((&matched.form, matched.kind)),
        // Canvas (receipt/image) jobs carry no duplex choice:
        // None leaves the driver default untouched.
        None,
        false,
        tray,
    )
    .ok_or_else(|| {
        GdiPrintError::setup(
            "PRINTER_DEVMODE_UNAVAILABLE: Windows driver could not apply the selected media form"
                .to_string(),
        )
    })?;
    let name = wide(printer);
    let init_ptr = devmode.as_ptr() as *const c_void;
    let dc = unsafe { CreateDCW(std::ptr::null(), name.as_ptr(), std::ptr::null(), init_ptr) };
    if dc.is_null() {
        return Err(GdiPrintError::setup(format!(
            "CreateDC for {printer:?}: {}",
            last_error()
        )));
    }
    let dc = PrinterDc(dc);
    let doc_name = wide(title);
    let info = DocInfoW {
        size: std::mem::size_of::<DocInfoW>() as i32,
        doc_name: doc_name.as_ptr(),
        output: std::ptr::null(),
        datatype: std::ptr::null(),
        fw_type: 0,
    };
    let started = unsafe { StartDocW(dc.0, &info) };
    if started <= 0 {
        return Err(GdiPrintError::setup(format!("StartDoc: {}", last_error())));
    }
    // Pagination decision: a receipt strip taller than the chosen
    // sheet is sliced across pages (the office/virtual printer
    // case). Roll and custom matches already carry the strip's own
    // height, so they keep printing as one continuous page.
    let paginate = !matched.form.roll
        && page_size.height_mm
            > matched.form.size.height_mm + print_core::paper::EXACT_TOLERANCE_MM;
    let mut sent = 0usize;
    let mut failure: Option<String> = None;
    if paginate {
        match print_canvas_pages(dc.0, canvas, copies.max(1), roll_width_mm) {
            Ok(pages) => sent = pages,
            Err(error) => failure = Some(error),
        }
    } else {
        // A short receipt on an office sheet (e.g. 80x120mm on A4) must
        // print at its real physical width instead of stretching 2.6x across
        // the full A4 width.
        let blit_kind = if !matched.form.roll
            && matched.kind == MatchKind::NearestLargerFit
            && page_size.width_mm <= 120.0
        {
            MatchKind::NearestLargerCentered
        } else {
            matched.kind
        };
        let page = print_render::RenderedPage {
            width_px: canvas.width,
            height_px: canvas.height,
            mode: print_render::RenderMode::Grayscale,
            dpi: 203,
            pixels: canvas.px.clone(),
        };
        let dib = crate::dib::build_dib(&page);
        for _ in 0..copies.max(1) {
            match blit_dib_page(dc.0, &dib, Some(page_size), blit_kind) {
                Ok(()) => sent += 1,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
    }
    if failure.is_some() {
        unsafe { AbortDoc(dc.0) };
    } else if unsafe { EndDoc(dc.0) } <= 0 {
        failure = Some(format!("EndDoc: {}", last_error()));
    }
    match failure {
        // The document started; earlier copies may be on paper even
        // though AbortDoc was called — ambiguous per ADR-0004.
        Some(error) => Err(GdiPrintError::mid_print(error)),
        None => Ok(sent),
    }
}

/// Print a tall canvas (a receipt strip) across as many sheet
/// pages as its height needs: the image is scaled ONCE to the
/// page's printable width, then sliced into page-height row
/// chunks printed top-aligned, page after page — the same thing
/// a browser does with a long receipt on A4. Returns the number
/// of pages printed.
fn print_canvas_pages(
    dc: Hdc,
    canvas: &print_render::bitmap::Canvas,
    copies: u16,
    max_width_mm: Option<f64>,
) -> Result<usize, String> {
    let area_w = unsafe { GetDeviceCaps(dc, HORZRES) };
    let area_h = unsafe { GetDeviceCaps(dc, VERTRES) };
    if area_w <= 0 || area_h <= 0 || canvas.width == 0 {
        return Err(format!("GetDeviceCaps: {}", last_error()));
    }
    let target_w = match max_width_mm {
        Some(mm) if mm > 0.0 && mm <= 120.0 => {
            let dpi_x = unsafe { GetDeviceCaps(dc, LOGPIXELSX) };
            if dpi_x > 0 {
                (((mm / 25.4) * dpi_x as f64).round() as i32).clamp(1, area_w)
            } else {
                area_w
            }
        }
        _ => area_w,
    };
    let scale = target_w as f64 / canvas.width as f64;
    let rows_per_page = ((area_h as f64 / scale).floor() as u32).max(1);
    let chunks = print_render::bitmap::page_row_chunks(canvas.height, rows_per_page);
    if chunks.is_empty() {
        return Err("receipt image has no rows to print".to_string());
    }
    let mut sent = 0usize;
    for _ in 0..copies.max(1) {
        for (start, rows) in &chunks {
            let chunk = print_render::bitmap::slice_canvas_rows(canvas, *start, *rows);
            let page = print_render::RenderedPage {
                width_px: chunk.width,
                height_px: chunk.height,
                mode: print_render::RenderMode::Grayscale,
                dpi: 203,
                pixels: chunk.px.clone(),
            };
            let dib = crate::dib::build_dib(&page);
            let drawn_h = ((chunk.height as f64 * scale).round() as u32)
                .min(area_h as u32)
                .max(1);
            blit_dib_rect(dc, &dib, 0, 0, target_w as u32, drawn_h)?;
            sent += 1;
        }
    }
    Ok(sent)
}

/// Draw one DIB as a full-width, top-aligned page (StartPage ..
/// EndPage): the chunk blit for paginated receipt strips, where
/// vertical centering (fit_to_page) would misplace every slice.
fn blit_dib_rect(
    dc: Hdc,
    dib: &crate::dib::DibImage,
    x: i32,
    y: i32,
    dest_w: u32,
    dest_h: u32,
) -> Result<(), String> {
    if dib.width == 0 || dib.height == 0 || dib.bits.is_empty() || dest_w == 0 || dest_h == 0 {
        return Err("StretchDIBits: empty DIB or destination rectangle".to_string());
    }
    let (dw, dh, sw, sh) = (
        i32::try_from(dest_w).map_err(|_| "StretchDIBits: dest_w overflow".to_string())?,
        i32::try_from(dest_h).map_err(|_| "StretchDIBits: dest_h overflow".to_string())?,
        i32::try_from(dib.width).map_err(|_| "StretchDIBits: src_w overflow".to_string())?,
        i32::try_from(dib.height).map_err(|_| "StretchDIBits: src_h overflow".to_string())?,
    );
    if unsafe { StartPage(dc) } <= 0 {
        return Err(format!("StartPage: {}", last_error()));
    }
    let info = dib.bitmap_info();
    let drawn = unsafe {
        StretchDIBits(
            dc,
            x,
            y,
            dw,
            dh,
            0,
            0,
            sw,
            sh,
            dib.bits.as_ptr() as *const c_void,
            info.as_ptr() as *const c_void,
            DIB_RGB_COLORS,
            SRCCOPY,
        )
    };
    if drawn == GDI_ERROR || drawn == 0 {
        // Capture GetLastError before EndPage can overwrite it.
        let err = last_error();
        unsafe { EndPage(dc) };
        return Err(format!("StretchDIBits: {err}"));
    }
    if unsafe { EndPage(dc) } <= 0 {
        return Err(format!("EndPage: {}", last_error()));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn print_subjob_gdi(
    dc: Hdc,
    doc: &print_render::PdfDocument,
    title: &str,
    start_page: usize,
    page_count: usize,
    page_size: PageSize,
    match_kind: MatchKind,
    mode: print_render::RenderMode,
    dpi: u32,
) -> Result<usize, GdiPrintError> {
    let doc_name = wide(title);
    let info = DocInfoW {
        size: std::mem::size_of::<DocInfoW>() as i32,
        doc_name: doc_name.as_ptr(),
        output: std::ptr::null(),
        datatype: std::ptr::null(),
        fw_type: 0,
    };
    let started = unsafe { StartDocW(dc, &info) };
    if started <= 0 {
        // The document never started: nothing can have printed.
        return Err(GdiPrintError::setup(format!("StartDoc: {}", last_error())));
    }
    let mut sent = 0usize;
    let mut failure: Option<String> = None;
    for i in start_page..(start_page + page_count) {
        match print_one_page(dc, doc, i, page_size, match_kind, mode, dpi) {
            Ok(()) => sent += 1,
            Err(e) => {
                failure = Some(format!("page {}: {e}", i + 1));
                break;
            }
        }
    }
    if failure.is_some() {
        unsafe { AbortDoc(dc) };
    } else if unsafe { EndDoc(dc) } <= 0 {
        failure = Some(format!("EndDoc: {}", last_error()));
    }
    match failure {
        // The document started and earlier pages of it were ended:
        // output may exist even though AbortDoc was called, so the
        // failure is ambiguous for ADR-0004 purposes.
        Some(e) => Err(GdiPrintError::mid_print(e)),
        None => Ok(sent),
    }
}

fn print_one_page(
    dc: Hdc,
    doc: &print_render::PdfDocument,
    index: usize,
    page_size: PageSize,
    match_kind: MatchKind,
    mode: print_render::RenderMode,
    dpi: u32,
) -> Result<(), String> {
    let page = doc
        .render_page(index, mode, dpi)
        .map_err(|e| e.to_string())?;
    let dib = crate::dib::build_dib(&page);
    blit_dib_page(dc, &dib, Some(page_size), match_kind)
}

fn blit_dib_page(
    dc: Hdc,
    dib: &crate::dib::DibImage,
    page_size: Option<PageSize>,
    match_kind: MatchKind,
) -> Result<(), String> {
    let area_w = unsafe { GetDeviceCaps(dc, HORZRES) };
    let area_h = unsafe { GetDeviceCaps(dc, VERTRES) };
    if area_w <= 0 || area_h <= 0 {
        return Err(format!("GetDeviceCaps: {}", last_error()));
    }
    let (x, y, w, h) = match (match_kind, page_size) {
        (MatchKind::NearestLargerCentered, Some(ps)) => {
            let dpi_x = unsafe { GetDeviceCaps(dc, LOGPIXELSX) }.max(72) as f64;
            let dpi_y = unsafe { GetDeviceCaps(dc, LOGPIXELSY) }.max(72) as f64;
            let target_w = ((ps.width_mm / 25.4) * dpi_x).round().max(1.0) as u32;
            let target_h = ((ps.height_mm / 25.4) * dpi_y).round().max(1.0) as u32;
            if target_w <= area_w as u32 && target_h <= area_h as u32 {
                let x = ((area_w as u32 - target_w) / 2) as i32;
                let y = ((area_h as u32 - target_h) / 2) as i32;
                (x, y, target_w, target_h)
            } else {
                crate::dib::fit_to_page(target_w, target_h, area_w as u32, area_h as u32)
            }
        }
        _ => crate::dib::fit_to_page(dib.width, dib.height, area_w as u32, area_h as u32),
    };
    blit_dib_rect(dc, dib, x, y, w, h)
}

#[repr(C)]
struct PrinterInfo2W {
    server_name: *const u16,
    printer_name: *const u16,
    share_name: *const u16,
    port_name: *const u16,
    driver_name: *const u16,
    comment: *const u16,
    location: *const u16,
    devmode: *mut c_void,
    sep_file: *const u16,
    print_processor: *const u16,
    datatype: *const u16,
    parameters: *const u16,
    security_descriptor: *mut c_void,
    attributes: u32,
    priority: u32,
    default_priority: u32,
    start_time: u32,
    until_time: u32,
    status: u32,
    c_jobs: u32,
    average_ppm: u32,
}

#[link(name = "winspool")]
extern "system" {
    fn EnumPrintersW(
        flags: u32,
        name: *const u16,
        level: u32,
        printer_enum: *mut u8,
        cb_buf: u32,
        needed: *mut u32,
        returned: *mut u32,
    ) -> Bool;
    fn GetPrinterW(
        handle: Handle,
        level: u32,
        printer: *mut u8,
        cb_buf: u32,
        needed: *mut u32,
    ) -> Bool;
}

const PRINTER_ENUM_LOCAL: u32 = 0x0000_0002;
const PRINTER_ENUM_CONNECTIONS: u32 = 0x0000_0004;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

unsafe fn read_wide(ptr: *const u16) -> String {
    const MAX_WIDE_LEN: usize = 32_768;
    if ptr.is_null() {
        return String::new();
    }
    let mut units = Vec::new();
    for len in 0..MAX_WIDE_LEN {
        let ch = ptr.add(len).read_unaligned();
        if ch == 0 {
            break;
        }
        units.push(ch);
    }
    String::from_utf16_lossy(&units)
}

unsafe fn convert_info2(info: &PrinterInfo2W) -> crate::discovery::RawPrinterInfo2 {
    crate::discovery::RawPrinterInfo2 {
        printer_name: read_wide(info.printer_name),
        port_name: read_wide(info.port_name),
        driver_name: read_wide(info.driver_name),
        attributes: info.attributes,
        status: info.status,
    }
}

/// Discover all local and per-user connected printers (EnumPrinters
/// level 2). Two-call pattern: size query, then fill.
/// How long a discovery snapshot is shared between callers. The
/// worker path resolves a printer per job; 5 s keeps bursts cheap
/// while a (dis)appearing printer is noticed quickly. UI callers
/// layer their own longer cache on top.
const DISCOVERY_TTL: std::time::Duration = std::time::Duration::from_secs(5);
static DISCOVERY_CACHE: crate::discovery::DiscoveryCache = crate::discovery::DiscoveryCache::new();

pub fn discover_printers() -> Result<Vec<print_core::PrinterInfo>, String> {
    DISCOVERY_CACHE.get_or_discover(DISCOVERY_TTL, discover_printers_uncached)
}

fn discover_printers_uncached() -> Result<Vec<print_core::PrinterInfo>, String> {
    let mut needed: u32 = 0;
    let mut returned: u32 = 0;
    let probe_ok = unsafe {
        EnumPrintersW(
            PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS,
            std::ptr::null(),
            2,
            std::ptr::null_mut(),
            0,
            &mut needed,
            &mut returned,
        )
    };
    if needed == 0 {
        // A successful probe reporting no buffer is a genuinely empty
        // spooler; a FAILED probe is an error and must not masquerade
        // as "no printers installed". (Audit L8.)
        if probe_ok == 0 {
            return Err(last_error());
        }
        return Ok(Vec::new());
    }
    let mut buf = Vec::new();
    for attempt in 0..3 {
        let cap = needed;
        buf = aligned_byte_buf(cap as usize);
        let ok = unsafe {
            EnumPrintersW(
                PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS,
                std::ptr::null(),
                2,
                buf.as_mut_ptr() as *mut u8,
                cap,
                &mut needed,
                &mut returned,
            )
        };
        if ok != 0 {
            break;
        }
        let code = last_error_code();
        if code == ERROR_INSUFFICIENT_BUFFER && needed > cap && attempt < 2 {
            continue;
        }
        return Err(format!("win32 error {code}"));
    }
    let buf_bytes = buf.len() * std::mem::size_of::<u64>();
    let stride = std::mem::size_of::<PrinterInfo2W>();
    let max_entries = buf_bytes / stride;
    let count = (returned as usize).min(max_entries);
    let mut out = Vec::new();
    for i in 0..count {
        let info = unsafe {
            let ptr = (buf.as_ptr() as *const u8).add(i * stride) as *const PrinterInfo2W;
            let record = ptr.read_unaligned();
            convert_info2(&record)
        };
        if crate::discovery::is_usable(&info) {
            out.push(crate::discovery::convert(&info));
        }
    }
    Ok(out)
}

/// Spooler-reported status for one printer: open it and read its
/// level-2 status via GetPrinter. This is the driver/spooler's view
/// of the device (it can be delayed or incomplete), NOT a direct
/// physical sensor read (network printers use TCP 9100 DLE EOT / ZPL / TSPL
/// or IPP probes separately in `print-raw` / `print-ipp`).
pub fn printer_status(printer: &str) -> Result<print_core::PrinterStatus, String> {
    printer_health(printer).map(|h| h.primary_status())
}

/// Full multi-condition health observation for one printer (same
/// spooler read as [`printer_status`], without collapsing the flag
/// word to a single status). A failed read is an error, never a
/// fabricated `Ready`: callers must treat it as unknown/stale.
pub fn printer_health(printer: &str) -> Result<print_core::PrinterHealth, String> {
    let name = wide(printer);
    let mut raw_handle: Handle = std::ptr::null_mut();
    let ok = unsafe { OpenPrinterW(name.as_ptr(), &mut raw_handle, std::ptr::null_mut()) };
    if ok == 0 || raw_handle.is_null() {
        return Err(last_error());
    }
    let handle = PrinterHandle(raw_handle);
    unsafe {
        let mut needed: u32 = 0;
        GetPrinterW(handle.0, 2, std::ptr::null_mut(), 0, &mut needed);
        if needed == 0 {
            return Err(last_error());
        }
        let mut buf = Vec::new();
        for attempt in 0..3 {
            let cap = needed;
            buf = aligned_byte_buf(cap as usize);
            let ok2 = GetPrinterW(handle.0, 2, buf.as_mut_ptr() as *mut u8, cap, &mut needed);
            if ok2 != 0 {
                break;
            }
            let code = last_error_code();
            if code == ERROR_INSUFFICIENT_BUFFER && needed > cap && attempt < 2 {
                continue;
            }
            return Err(format!("win32 error {code}"));
        }
        if (needed as usize) < std::mem::size_of::<PrinterInfo2W>() {
            Err("GetPrinterW returned truncated PRINTER_INFO_2W".to_string())
        } else {
            let info = (buf.as_ptr() as *const PrinterInfo2W).read_unaligned();
            match crate::discovery::convert(&convert_info2(&info)).health {
                Some(health) => Ok(health),
                None => Err("spooler status produced no health observation".to_string()),
            }
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn last_error_code() -> u32 {
    unsafe {
        // GetLastError via kernel32, declared locally to keep the FFI
        // surface in one auditable place.
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLastError() -> u32;
        }
        GetLastError()
    }
}

fn last_error() -> String {
    format!("win32 error {}", last_error_code())
}

/// The system's default printer name.
pub fn default_printer_name() -> Result<String, String> {
    let mut size: u32 = 0;
    unsafe { GetDefaultPrinterW(std::ptr::null_mut(), &mut size) };
    if size == 0 {
        return Err(last_error());
    }
    let mut buf = Vec::new();
    for attempt in 0..3 {
        let cap = size;
        buf = vec![0u16; cap as usize];
        let ok = unsafe { GetDefaultPrinterW(buf.as_mut_ptr(), &mut size) };
        if ok != 0 {
            break;
        }
        let code = last_error_code();
        if code == ERROR_INSUFFICIENT_BUFFER && size > cap && attempt < 2 {
            continue;
        }
        return Err(format!("win32 error {code}"));
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(String::from_utf16_lossy(&buf[..end]))
}

/// Paper forms for a printer, via DeviceCapabilities (DC_PAPERS,
/// DC_PAPERSIZE, and DC_PAPERNAMES). Sizes come back in tenths of a
/// millimetre (POINT structs), IDs in WORDs, and names in 64-WCHAR blocks.
pub fn paper_forms(printer: &str) -> Result<Vec<Form>, String> {
    let name = wide(printer);
    let count = unsafe {
        DeviceCapabilitiesW(
            name.as_ptr(),
            std::ptr::null(),
            DC_PAPERS,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if count < 0 {
        return Err(last_error());
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    if count > 4096 {
        return Err(format!(
            "DeviceCapabilitiesW(DC_PAPERS) returned excessive count {count}"
        ));
    }
    let n_ids = count as usize;
    let mut paper_ids = vec![0u16; n_ids];
    let got_ids = unsafe {
        DeviceCapabilitiesW(
            name.as_ptr(),
            std::ptr::null(),
            DC_PAPERS,
            paper_ids.as_mut_ptr() as *mut c_void,
            std::ptr::null(),
        )
    };
    let size_count = unsafe {
        DeviceCapabilitiesW(
            name.as_ptr(),
            std::ptr::null(),
            DC_PAPERSIZE,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if size_count <= 0 {
        return Err(last_error());
    }
    if size_count > 4096 {
        return Err(format!(
            "DeviceCapabilitiesW(DC_PAPERSIZE) returned excessive count {size_count}"
        ));
    }
    let n_sizes = size_count as usize;
    let mut sizes = vec![0i32; n_sizes * 2]; // POINT is two LONGs (i32)
    let got = unsafe {
        DeviceCapabilitiesW(
            name.as_ptr(),
            std::ptr::null(),
            DC_PAPERSIZE,
            sizes.as_mut_ptr() as *mut c_void,
            std::ptr::null(),
        )
    };
    if got <= 0 {
        return Err(last_error());
    }
    let valid_count = (got as usize).min(n_sizes);
    // Query DC_PAPERNAMES (16): each entry is a 64-WCHAR block.
    let raw_name_count = unsafe {
        DeviceCapabilitiesW(
            name.as_ptr(),
            std::ptr::null(),
            DC_PAPERNAMES,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    let n_names = if (1..=4096).contains(&raw_name_count) {
        raw_name_count as usize
    } else {
        0
    };
    let mut name_blocks = vec![0u16; n_names * 64];
    let got_names = if n_names == 0 {
        0
    } else {
        unsafe {
            DeviceCapabilitiesW(
                name.as_ptr(),
                std::ptr::null(),
                DC_PAPERNAMES,
                name_blocks.as_mut_ptr() as *mut c_void,
                std::ptr::null(),
            )
        }
    };
    let valid_names = (got_names.max(0) as usize).min(n_names);
    let valid_ids = (got_ids.max(0) as usize).min(n_ids);
    let mut forms = Vec::with_capacity(valid_count);
    for i in 0..valid_count {
        let w_mm = sizes[i * 2] as f64 / 10.0;
        let h_mm = sizes[i * 2 + 1] as f64 / 10.0;
        let raw_name = if i < valid_names {
            let block = &name_blocks[i * 64..(i + 1) * 64];
            let end = block.iter().position(|&c| c == 0).unwrap_or(64);
            String::from_utf16_lossy(&block[..end])
        } else {
            String::new()
        };
        let lower_name = raw_name.to_ascii_lowercase();
        let named_roll = lower_name.contains("roll")
            || lower_name.contains("receipt")
            || lower_name.contains("continuous");
        if w_mm > 0.0 && (h_mm > 0.0 || named_roll) {
            let mut form = crate::discovery::classify_form(&raw_name, w_mm, h_mm);
            if i < valid_ids && paper_ids[i] > 0 {
                form.paper_id = Some(paper_ids[i]);
            }
            forms.push(form);
        }
    }
    Ok(forms)
}

/// Query capabilities reported by the installed Windows printer driver.
pub fn printer_capabilities(printer: &str) -> Result<print_core::PrinterCapabilities, String> {
    let name = wide(printer);
    let forms = paper_forms(printer)?;
    let count = |capability: u16| unsafe {
        DeviceCapabilitiesW(
            name.as_ptr(),
            std::ptr::null(),
            capability,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    // DeviceCapabilitiesW returns -1 on error, which the API does
    // not distinguish from "unsupported". A -1 is therefore
    // reported as "unknown" (None) to the provenance-aware
    // constructor instead of being clamped into a fabricated
    // concrete value (e.g. max_copies=1 presented as a device fact).
    let duplex_raw = count(DC_DUPLEX);
    let duplex = if duplex_raw < 0 {
        None
    } else {
        Some(duplex_raw > 0)
    };
    let color_raw = count(DC_COLORDEVICE);
    let color = if color_raw < 0 {
        None
    } else {
        Some(color_raw > 0)
    };
    let copies_raw = count(DC_COPIES);
    let max_copies = if copies_raw < 0 {
        None
    } else {
        Some(copies_raw.clamp(1, u16::MAX as i32) as u16)
    };

    let raw_res_count = count(DC_ENUMRESOLUTIONS);
    let resolution_count = if (1..=4096).contains(&raw_res_count) {
        raw_res_count as usize
    } else {
        0
    };
    let mut resolution_pairs = vec![0i32; resolution_count * 2];
    let got_resolutions = if resolution_count == 0 {
        0
    } else {
        unsafe {
            DeviceCapabilitiesW(
                name.as_ptr(),
                std::ptr::null(),
                DC_ENUMRESOLUTIONS,
                resolution_pairs.as_mut_ptr() as *mut c_void,
                std::ptr::null(),
            )
        }
    };
    let mut resolutions_dpi: Vec<u32> = (0..(got_resolutions.max(0) as usize)
        .min(resolution_count))
        .filter_map(|i| {
            let x = resolution_pairs[i * 2];
            let y = resolution_pairs[i * 2 + 1];
            (x > 0 && y > 0).then_some(x.min(y) as u32)
        })
        .collect();
    resolutions_dpi.sort_unstable();
    resolutions_dpi.dedup();

    let raw_bin_count = count(DC_BINNAMES);
    let bin_count = if (1..=4096).contains(&raw_bin_count) {
        raw_bin_count as usize
    } else {
        0
    };
    let mut bin_names = vec![0u16; bin_count * 24];
    let got_bin_names = if bin_count == 0 {
        0
    } else {
        unsafe {
            DeviceCapabilitiesW(
                name.as_ptr(),
                std::ptr::null(),
                DC_BINNAMES,
                bin_names.as_mut_ptr() as *mut c_void,
                std::ptr::null(),
            )
        }
    };
    let valid_bins = (got_bin_names.max(0) as usize).min(bin_count);
    let bins: Vec<String> = (0..valid_bins)
        .filter_map(|i| {
            let block = &bin_names[i * 24..(i + 1) * 24];
            let end = block.iter().position(|v| *v == 0).unwrap_or(24);
            let name = String::from_utf16_lossy(&block[..end]).trim().to_string();
            (!name.is_empty()).then_some(name)
        })
        .collect();

    // DC_MINEXTENT and DC_MAXEXTENT return packed width/length in the
    // function result (LOWORD / HIWORD); pOutput is not used for these flags.
    let extent = |capability: u16| -> Option<(f64, f64)> {
        let packed = count(capability);
        if packed <= 0 {
            return None;
        }
        let packed = packed as u32;
        let width_tenths_mm = (packed & 0xffff) as u16;
        let length_tenths_mm = (packed >> 16) as u16;
        if width_tenths_mm == 0 || length_tenths_mm == 0 {
            return None;
        }
        Some((
            width_tenths_mm as f64 / 10.0,
            length_tenths_mm as f64 / 10.0,
        ))
    };
    let min_custom_mm = extent(DC_MINEXTENT);
    let max_custom_mm = extent(DC_MAXEXTENT);
    let bins_reported = !bins.is_empty();
    Ok(print_core::PrinterCapabilities::from_driver_report(
        forms,
        resolutions_dpi,
        duplex,
        color,
        max_copies,
        bins,
        bins_reported,
        min_custom_mm,
        max_custom_mm,
    ))
}

/// Submit raw bytes (ESC/POS, ZPL, TSPL) to a printer through the spooler
/// with the RAW datatype. Returns the spooler job id.
pub fn print_raw(printer: &str, title: &str, bytes: &[u8]) -> Result<u32, String> {
    print_raw_phased(printer, title, bytes).map_err(|e| e.message)
}

/// `print_raw` with the failure phase attached (see RawSendError).
pub fn print_raw_phased(printer: &str, title: &str, bytes: &[u8]) -> Result<u32, RawSendError> {
    let pre = |message: String| RawSendError {
        pre_write: true,
        message,
    };
    let name = wide(printer);
    let mut raw_handle: Handle = std::ptr::null_mut();
    let ok = unsafe { OpenPrinterW(name.as_ptr(), &mut raw_handle, std::ptr::null_mut()) };
    if ok == 0 || raw_handle.is_null() {
        return Err(pre(last_error()));
    }
    let handle = PrinterHandle(raw_handle);
    unsafe {
        let doc_name = wide(title);
        let datatype = wide("RAW");
        let doc = DocInfo1W {
            doc_name: doc_name.as_ptr(),
            output_file: std::ptr::null(),
            datatype: datatype.as_ptr(),
        };
        let job_id = StartDocPrinterW(handle.0, 1, &doc);
        if job_id == 0 {
            Err(pre(last_error()))
        } else if StartPagePrinter(handle.0) == 0 {
            // Capture the error BEFORE AbortPrinter can clobber it.
            // No payload byte was written yet: still pre-write.
            let err = last_error();
            AbortPrinter(handle.0);
            Err(pre(err))
        } else {
            // WritePrinter may accept fewer bytes than requested;
            // loop until the whole payload is in the spooler. On any
            // failure the job is ABORTED, never ended: EndDocPrinter
            // would commit a truncated stream as a complete document
            // while the caller sees an error and retries — printing a
            // truncated copy plus a duplicate. (Audit H1.)
            let mut offset = 0usize;
            let mut failure: Option<String> = None;
            while offset < bytes.len() {
                let chunk = &bytes[offset..];
                let mut written: u32 = 0;
                let w_ok = WritePrinter(
                    handle.0,
                    chunk.as_ptr() as *const c_void,
                    u32::try_from(chunk.len()).unwrap_or(u32::MAX),
                    &mut written,
                );
                if w_ok == 0 || written == 0 {
                    let err = last_error();
                    failure = Some(format!(
                        "WritePrinter failed at byte {offset} of {}: {err}",
                        bytes.len()
                    ));
                    break;
                }
                offset += (written as usize).min(chunk.len());
            }
            if let Some(err) = failure {
                AbortPrinter(handle.0);
                // A write-phase failure is ambiguous by definition
                // (ADR-0004): bytes may already be on paper.
                Err(RawSendError {
                    pre_write: false,
                    message: err,
                })
            } else if EndPagePrinter(handle.0) == 0 || EndDocPrinter(handle.0) == 0 {
                let err = last_error();
                AbortPrinter(handle.0);
                Err(RawSendError {
                    pre_write: false,
                    message: format!("EndDocPrinter failed: {err}"),
                })
            } else {
                Ok(job_id)
            }
        }
    }
}

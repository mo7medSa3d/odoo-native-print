//! Win32 FFI for winspool.drv and gdi32.dll. Only simple, stable buffer APIs are used.
//! All wide strings are NUL-terminated UTF-16; every handle opened here is
//! closed on every path out of its function.

use crate::GdiJobOptions;
use print_core::paper::{match_paper, match_paper_auto, Form, MatchKind, ScalePolicy};
use print_core::PageSize;
use print_render::split_subjobs;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;

type Handle = *mut c_void;
type Bool = i32;

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
    if count <= 0 {
        return None;
    }
    let n = count as usize;
    let mut bin_ids = vec![0u16; n];
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
    let mut bin_names = vec![0u16; n * 24];
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
    let count_ok = (got_ids as usize).min(got_names as usize).min(n);
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
fn build_printer_devmode(
    printer: &str,
    page_size: PageSize,
    matched_form: Option<(&Form, MatchKind)>,
    duplex: bool,
    color: bool,
    tray: Option<&str>,
) -> Option<Vec<u8>> {
    let name = wide(printer);
    let mut handle: Handle = std::ptr::null_mut();
    let ok = unsafe { OpenPrinterW(name.as_ptr(), &mut handle, std::ptr::null_mut()) };
    if ok == 0 || handle.is_null() {
        return None;
    }
    let result = unsafe {
        let needed = DocumentPropertiesW(
            std::ptr::null_mut(),
            handle,
            name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null(),
            0,
        );
        if needed < std::mem::size_of::<DevModePrefixW>() as i32 {
            None
        } else {
            let mut buf = vec![0u8; needed as usize];
            let got = DocumentPropertiesW(
                std::ptr::null_mut(),
                handle,
                name.as_ptr(),
                buf.as_mut_ptr(),
                std::ptr::null(),
                DM_OUT_BUFFER,
            );
            if got < 0 {
                None
            } else {
                #[allow(clippy::cast_ptr_alignment)]
                let dm = &mut *(buf.as_mut_ptr() as *mut DevModePrefixW);
                // Orientation
                dm.dm_orientation = if page_size.is_landscape() {
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
                    Some((
                        form,
                        MatchKind::Exact
                        | MatchKind::NearestLargerCentered
                        | MatchKind::NearestLargerFit,
                    )) if !form.name.is_empty() && form.name != "Custom" => {
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
                        dm.dm_fields |= DM_PAPERSIZE | DM_PAPERWIDTH | DM_PAPERLENGTH;
                    }
                }

                // Input tray / bin selection
                if let Some(t) = tray {
                    if let Some(bin_id) = resolve_printer_bin_id(&name, t) {
                        dm.dm_default_source = bin_id;
                        dm.dm_fields |= DM_DEFAULTSOURCE;
                    }
                }

                // Duplex & Color
                dm.dm_duplex = if duplex {
                    DMDUP_VERTICAL
                } else {
                    DMDUP_SIMPLEX
                };
                dm.dm_fields |= DM_DUPLEX;
                dm.dm_color = if color {
                    DMCOLOR_COLOR
                } else {
                    DMCOLOR_MONOCHROME
                };
                dm.dm_fields |= DM_COLOR;

                let merged = DocumentPropertiesW(
                    std::ptr::null_mut(),
                    handle,
                    name.as_ptr(),
                    buf.as_mut_ptr(),
                    buf.as_ptr(),
                    DM_IN_BUFFER | DM_OUT_BUFFER,
                );
                if merged < 0 {
                    None
                } else {
                    Some(buf)
                }
            }
        }
    };
    unsafe { ClosePrinter(handle) };
    result
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
        duplex: false,
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
    let pdfium = print_render::Pdfium::load().map_err(|e| e.to_string())?;
    let doc = pdfium.load_document(pdf_bytes).map_err(|e| e.to_string())?;
    let pages = doc.page_count();
    if pages == 0 {
        return Err("PDF has no pages".to_string());
    }
    let mut page_sizes = Vec::with_capacity(pages);
    for i in 0..pages {
        page_sizes.push(doc.page_size_mm(i).map_err(|e| e.to_string())?);
    }
    let subjobs = split_subjobs(&page_sizes);
    let capabilities = printer_capabilities(printer).map_err(|e| e.to_string())?;
    let name = wide(printer);
    let mut total_sent = 0usize;

    for _copy in 0..opts.copies.max(1) {
        for sub in &subjobs {
            let matched = if capabilities.forms.is_empty() && !capabilities.custom_size_supported {
                None
            } else if opts.scale_policy == ScalePolicy::CustomAllowed {
                Some(match_paper_auto(sub.size, &capabilities).map_err(|e| e.to_string())?)
            } else {
                Some(
                    match_paper(sub.size, &capabilities.forms, opts.scale_policy)
                        .map_err(|e| e.to_string())?,
                )
            };
            let match_ref = matched.as_ref().map(|m| (&m.form, m.kind));
            let match_kind = matched.as_ref().map(|m| m.kind).unwrap_or(MatchKind::Exact);
            let media_size = matched
                .as_ref()
                .map(|selection| selection.form.size)
                .unwrap_or(sub.size);
            let devmode = build_printer_devmode(
                printer,
                media_size,
                match_ref,
                opts.duplex,
                opts.color,
                opts.tray.as_deref(),
            );
            let init_ptr = devmode
                .as_ref()
                .map(|b| b.as_ptr() as *const c_void)
                .unwrap_or(std::ptr::null());
            let dc =
                unsafe { CreateDCW(std::ptr::null(), name.as_ptr(), std::ptr::null(), init_ptr) };
            if dc.is_null() {
                return Err(format!("CreateDC for {printer:?}: {}", last_error()));
            }
            let sub_result = print_subjob_gdi(
                dc,
                &doc,
                title,
                sub.start_page,
                sub.page_count,
                sub.size,
                match_kind,
                opts.mode,
                opts.dpi,
            );
            unsafe { DeleteDC(dc) };
            total_sent += sub_result?;
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
    if canvas.width == 0 || canvas.height == 0 {
        return Err("empty image canvas".to_string());
    }
    let page = print_render::RenderedPage {
        width_px: canvas.width,
        height_px: canvas.height,
        mode: print_render::RenderMode::Grayscale,
        dpi: 203,
        pixels: canvas.px.clone(),
    };
    let dib = crate::dib::build_dib(&page);

    // POS/kitchen PNG jobs that use GDI must obey the same media contract as
    // PDF jobs: derive the physical page geometry from the 203-DPI raster,
    // then select an exact/roll/custom/larger media form from the driver.
    let page_size = PageSize::new(
        canvas.width as f64 * 25.4 / 203.0,
        canvas.height as f64 * 25.4 / 203.0,
    );
    let capabilities = printer_capabilities(printer)?;
    if capabilities.forms.is_empty() && !capabilities.custom_size_supported {
        return Err(
            "PRINTER_MEDIA_UNAVAILABLE: Windows driver reported no supported forms or custom-size bounds"
                .to_string(),
        );
    }
    let matched = match_paper_auto(page_size, &capabilities).map_err(|error| {
        format!("PAPER_MISMATCH: no safe media form for the receipt image: {error}")
    })?;
    let media_page_size = matched.form.size;
    let devmode = build_printer_devmode(
        printer,
        media_page_size,
        Some((&matched.form, matched.kind)),
        false,
        false,
        None,
    )
    .ok_or_else(|| {
        "PRINTER_DEVMODE_UNAVAILABLE: Windows driver could not apply the selected media form"
            .to_string()
    })?;
    let name = wide(printer);
    let init_ptr = devmode.as_ptr() as *const c_void;
    let dc = unsafe { CreateDCW(std::ptr::null(), name.as_ptr(), std::ptr::null(), init_ptr) };
    if dc.is_null() {
        return Err(format!("CreateDC for {printer:?}: {}", last_error()));
    }
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
        let err = last_error();
        unsafe { DeleteDC(dc) };
        return Err(format!("StartDoc: {err}"));
    }
    let mut sent = 0usize;
    let mut failure: Option<String> = None;
    for _ in 0..copies.max(1) {
        match blit_dib_page(dc, &dib, Some(page_size), matched.kind) {
            Ok(()) => sent += 1,
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    if failure.is_some() {
        unsafe { AbortDoc(dc) };
    } else {
        unsafe { EndDoc(dc) };
    }
    unsafe { DeleteDC(dc) };
    match failure {
        Some(error) => Err(error),
        None => Ok(sent),
    }
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
) -> Result<usize, String> {
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
        return Err(format!("StartDoc: {}", last_error()));
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
    } else {
        unsafe { EndDoc(dc) };
    }
    match failure {
        Some(e) => Err(e),
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
            let w = target_w.min(area_w as u32);
            let h = target_h.min(area_h as u32);
            let x = ((area_w as u32).saturating_sub(w) / 2) as i32;
            let y = ((area_h as u32).saturating_sub(h) / 2) as i32;
            (x, y, w, h)
        }
        _ => crate::dib::fit_to_page(dib.width, dib.height, area_w as u32, area_h as u32),
    };
    if unsafe { StartPage(dc) } <= 0 {
        return Err(format!("StartPage: {}", last_error()));
    }
    let info = dib.bitmap_info();
    let drawn = unsafe {
        StretchDIBits(
            dc,
            x,
            y,
            w as i32,
            h as i32,
            0,
            0,
            dib.width as i32,
            dib.height as i32,
            dib.bits.as_ptr() as *const c_void,
            info.as_ptr() as *const c_void,
            DIB_RGB_COLORS,
            SRCCOPY,
        )
    };
    let end = unsafe { EndPage(dc) };
    if drawn == GDI_ERROR || drawn == 0 {
        return Err(format!("StretchDIBits: {}", last_error()));
    }
    if end <= 0 {
        return Err(format!("EndPage: {}", last_error()));
    }
    Ok(())
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

unsafe fn read_wide(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
}

unsafe fn convert_info2(info: &PrinterInfo2W) -> crate::discovery::RawPrinterInfo2 {
    crate::discovery::RawPrinterInfo2 {
        server_name: read_wide(info.server_name),
        printer_name: read_wide(info.printer_name),
        share_name: read_wide(info.share_name),
        port_name: read_wide(info.port_name),
        driver_name: read_wide(info.driver_name),
        attributes: info.attributes,
        status: info.status,
        job_count: info.c_jobs,
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
    let mut buf = vec![0u8; needed as usize];
    let ok = unsafe {
        EnumPrintersW(
            PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS,
            std::ptr::null(),
            2,
            buf.as_mut_ptr(),
            needed,
            &mut needed,
            &mut returned,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    let stride = std::mem::size_of::<PrinterInfo2W>();
    let mut out = Vec::new();
    for i in 0..returned as usize {
        let info = unsafe {
            let ptr = buf.as_ptr().add(i * stride) as *const PrinterInfo2W;
            convert_info2(&*ptr)
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
/// physical sensor read. (A DLE EOT status library exists in
/// print-raw but is not wired into the app, so no physical sensor
/// state is sensed today.)
pub fn printer_status(printer: &str) -> Result<print_core::PrinterStatus, String> {
    let name = wide(printer);
    let mut handle: Handle = std::ptr::null_mut();
    let ok = unsafe { OpenPrinterW(name.as_ptr(), &mut handle, std::ptr::null_mut()) };
    if ok == 0 || handle.is_null() {
        return Err(last_error());
    }
    let result = unsafe {
        let mut needed: u32 = 0;
        GetPrinterW(handle, 2, std::ptr::null_mut(), 0, &mut needed);
        if needed == 0 {
            Err(last_error())
        } else {
            let mut buf = vec![0u8; needed as usize];
            let ok2 = GetPrinterW(handle, 2, buf.as_mut_ptr(), needed, &mut needed);
            if ok2 == 0 {
                Err(last_error())
            } else {
                let info = &*(buf.as_ptr() as *const PrinterInfo2W);
                Ok(crate::discovery::convert(&convert_info2(info)).status)
            }
        }
    };
    unsafe { ClosePrinter(handle) };
    result
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn last_error() -> String {
    format!("win32 error {}", unsafe {
        // GetLastError via kernel32, declared locally to keep the FFI
        // surface in one auditable place.
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLastError() -> u32;
        }
        GetLastError()
    })
}

/// The system's default printer name.
pub fn default_printer_name() -> Result<String, String> {
    let mut size: u32 = 0;
    unsafe { GetDefaultPrinterW(std::ptr::null_mut(), &mut size) };
    if size == 0 {
        return Err(last_error());
    }
    let mut buf = vec![0u16; size as usize];
    let ok = unsafe { GetDefaultPrinterW(buf.as_mut_ptr(), &mut size) };
    if ok == 0 {
        return Err(last_error());
    }
    Ok(String::from_utf16_lossy(&buf)
        .trim_end_matches('\0')
        .to_string())
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
    if count <= 0 {
        return Err(last_error());
    }
    let n = count as usize;
    let mut paper_ids = vec![0u16; n];
    let got_ids = unsafe {
        DeviceCapabilitiesW(
            name.as_ptr(),
            std::ptr::null(),
            DC_PAPERS,
            paper_ids.as_mut_ptr() as *mut c_void,
            std::ptr::null(),
        )
    };
    let mut sizes = vec![0i32; n * 2]; // POINT is two LONGs (i32)
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
    let valid_count = (got as usize).min(n);
    // Query DC_PAPERNAMES (16): each entry is a 64-WCHAR block.
    let mut name_blocks = vec![0u16; n * 64];
    let got_names = unsafe {
        DeviceCapabilitiesW(
            name.as_ptr(),
            std::ptr::null(),
            DC_PAPERNAMES,
            name_blocks.as_mut_ptr() as *mut c_void,
            std::ptr::null(),
        )
    };
    let mut forms = Vec::with_capacity(valid_count);
    for i in 0..valid_count {
        let w_mm = sizes[i * 2] as f64 / 10.0;
        let h_mm = sizes[i * 2 + 1] as f64 / 10.0;
        let raw_name = if got_names > 0 && i < (got_names as usize) {
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
            if got_ids > 0 && i < (got_ids as usize) && paper_ids[i] > 0 {
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
    let duplex = count(DC_DUPLEX) > 0;
    let color = count(DC_COLORDEVICE) > 0;
    let max_copies = count(DC_COPIES).clamp(1, u16::MAX as i32) as u16;

    let resolution_count = count(DC_ENUMRESOLUTIONS).clamp(0, 128) as usize;
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

    let bin_count = count(DC_BINS).clamp(0, 128) as usize;
    let mut bin_ids = vec![0u16; bin_count];
    let got_bins = if bin_count == 0 {
        0
    } else {
        unsafe {
            DeviceCapabilitiesW(
                name.as_ptr(),
                std::ptr::null(),
                DC_BINS,
                bin_ids.as_mut_ptr() as *mut c_void,
                std::ptr::null(),
            )
        }
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
    let valid_bins = (got_bins.max(0) as usize)
        .min(got_bin_names.max(0) as usize)
        .min(bin_count);
    let bins = (0..valid_bins)
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
    Ok(print_core::PrinterCapabilities {
        forms,
        resolutions_dpi,
        duplex,
        color,
        bins,
        max_copies,
        custom_size_supported: min_custom_mm.is_some() && max_custom_mm.is_some(),
        min_custom_mm,
        max_custom_mm,
    })
}

/// Submit raw bytes (ESC/POS, ZPL, TSPL) to a printer through the spooler
/// with the RAW datatype. Returns the spooler job id.
pub fn print_raw(printer: &str, title: &str, bytes: &[u8]) -> Result<u32, String> {
    let name = wide(printer);
    let mut handle: Handle = std::ptr::null_mut();
    let ok = unsafe { OpenPrinterW(name.as_ptr(), &mut handle, std::ptr::null_mut()) };
    if ok == 0 || handle.is_null() {
        return Err(last_error());
    }
    let result = unsafe {
        let doc_name = wide(title);
        let datatype = wide("RAW");
        let doc = DocInfo1W {
            doc_name: doc_name.as_ptr(),
            output_file: std::ptr::null(),
            datatype: datatype.as_ptr(),
        };
        let job_id = StartDocPrinterW(handle, 1, &doc);
        if job_id == 0 {
            Err(last_error())
        } else if StartPagePrinter(handle) == 0 {
            // Capture the error BEFORE EndDocPrinter can clobber it.
            let err = last_error();
            EndDocPrinter(handle);
            Err(err)
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
                    handle,
                    chunk.as_ptr() as *const c_void,
                    u32::try_from(chunk.len()).unwrap_or(u32::MAX),
                    &mut written,
                );
                let err = last_error();
                if w_ok == 0 || written == 0 {
                    failure = Some(format!(
                        "WritePrinter failed at byte {offset} of {}: {err}",
                        bytes.len()
                    ));
                    break;
                }
                offset += written as usize;
            }
            if let Some(err) = failure {
                AbortPrinter(handle);
                Err(err)
            } else {
                EndPagePrinter(handle);
                EndDocPrinter(handle);
                Ok(job_id)
            }
        }
    };
    unsafe { ClosePrinter(handle) };
    result
}

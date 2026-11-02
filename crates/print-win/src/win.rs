//! Win32 FFI for winspool.drv. Only simple, stable buffer APIs are used.
//! All wide strings are NUL-terminated UTF-16; every handle opened here is
//! closed on every path out of its function.

use print_core::paper::Form;
use print_core::PageSize;
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
}

const DC_PAPERS: u16 = 2;
const DC_PAPERSIZE: u16 = 3;

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
pub fn discover_printers() -> Result<Vec<print_core::PrinterInfo>, String> {
    let mut needed: u32 = 0;
    let mut returned: u32 = 0;
    unsafe {
        EnumPrintersW(
            PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS,
            std::ptr::null(),
            2,
            std::ptr::null_mut(),
            0,
            &mut needed,
            &mut returned,
        );
    }
    if needed == 0 {
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

/// Sensor read for one printer: open it and read its level-2 status
/// (paper out, jam, door open, toner, offline...) via GetPrinter.
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

/// Paper forms for a printer, via DeviceCapabilities. Sizes come back in
/// tenths of a millimetre (POINT structs) and are converted to mm here.
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
    // Names are synthesised from sizes here; DC_PAPERNAMES name resolution
    // joins in the discovery work. Sizes are what the ladder consumes.
    let mut forms = Vec::with_capacity(n);
    for i in 0..n {
        let w_mm = sizes[i * 2] as f64 / 10.0;
        let h_mm = sizes[i * 2 + 1] as f64 / 10.0;
        if w_mm > 0.0 && h_mm > 0.0 {
            forms.push(Form::sheet(
                &format!("{w_mm:.0}x{h_mm:.0}mm"),
                PageSize::new(w_mm, h_mm),
            ));
        }
    }
    Ok(forms)
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
            EndDocPrinter(handle);
            Err(last_error())
        } else {
            let mut written: u32 = 0;
            let w_ok = WritePrinter(
                handle,
                bytes.as_ptr() as *const c_void,
                bytes.len() as u32,
                &mut written,
            );
            EndPagePrinter(handle);
            EndDocPrinter(handle);
            if w_ok == 0 || written as usize != bytes.len() {
                Err(format!(
                    "WritePrinter wrote {written} of {} bytes: {}",
                    bytes.len(),
                    last_error()
                ))
            } else {
                Ok(job_id)
            }
        }
    };
    unsafe { ClosePrinter(handle) };
    result
}

//! Runtime PDF rendering via PDFium, bound dynamically at runtime.
//!
//! Why direct FFI through `libloading` instead of the `pdfium-render`
//! crate: the library is a *bundled resource* whose location is only
//! known at runtime (installer resource dir beside the exe, or the
//! `NATIVE_PRINT_PDFIUM_PATH` override), and the FFI surface needed
//! here is ~15 stable C entry points. Declaring them in this one file
//! keeps the supply chain to a single small dependency (`libloading`)
//! and makes every unsafe call auditable in place. It also avoids the
//! version coupling of a generated-bindings crate: the pinned PDFium
//! build is recorded in docs/pdfium.md and can move independently.
//!
//! Failure policy: a missing or unloadable library is a loud error
//! ([`PdfiumError::LibraryNotFound`] lists every path searched). There
//! is no silent fallback renderer — a print client that quietly
//! substitutes a worse render would produce wrong paper, which this
//! project treats as worse than a failed job (ADR-0003 spirit).
//!
//! Threading: PDFium is not thread-safe. Every entry point is
//! serialized behind a process-wide mutex, and library init/destroy is
//! reference-counted across [`Pdfium`] instances.

use std::ffi::{c_int, c_ulong, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use print_core::PageSize;
use thiserror::Error;

use crate::plan::{plan_render, RenderMode, PER_PAGE_CEILING_BYTES};

/// Env override: path to the PDFium library file, or to a directory
/// containing it. Checked before every bundled/system location.
pub const ENV_PDFIUM_PATH: &str = "NATIVE_PRINT_PDFIUM_PATH";

#[cfg(windows)]
const LIB_NAMES: &[&str] = &["pdfium.dll"];
#[cfg(target_os = "macos")]
const LIB_NAMES: &[&str] = &["libpdfium.dylib", "libpdfium.so"];
#[cfg(all(not(windows), not(target_os = "macos")))]
const LIB_NAMES: &[&str] = &["libpdfium.so"];

#[derive(Debug, Error)]
pub enum PdfiumError {
    #[error("PDFium library not found; searched: {}", searched.join(", "))]
    LibraryNotFound { searched: Vec<String> },
    #[error("PDFium library failed to load from {path}: {source}")]
    LoadFailed {
        path: String,
        source: libloading::Error,
    },
    #[error("PDFium library is missing the required symbol {0}")]
    MissingSymbol(String),
    #[error("PDFium rejected the document (FPDF_GetLastError={code}): {detail}")]
    InvalidDocument { code: u32, detail: String },
    #[error("page index {0} out of range")]
    PageOutOfRange(usize),
    #[error(
        "render of one page needs {needed} bytes, over the per-page ceiling of {ceiling} bytes; \
         refusing instead of allocating unbounded memory"
    )]
    CeilingExceeded { needed: u64, ceiling: u64 },
    #[error("PDFium failed to create a {0}x{1} render bitmap")]
    BitmapFailed(u32, u32),
}

// --- FFI surface (the stable public PDFium API, fpdfview.h) ----------

type FpdfDocument = *mut c_void;
type FpdfPage = *mut c_void;
type FpdfBitmap = *mut c_void;

type FnInitLibrary = unsafe extern "C" fn();
type FnDestroyLibrary = unsafe extern "C" fn();
type FnLoadMemDocument = unsafe extern "C" fn(*const u8, c_int, *const i8) -> FpdfDocument;
type FnGetLastError = unsafe extern "C" fn() -> c_ulong;
type FnGetPageCount = unsafe extern "C" fn(FpdfDocument) -> c_int;
type FnCloseDocument = unsafe extern "C" fn(FpdfDocument);
type FnLoadPage = unsafe extern "C" fn(FpdfDocument, c_int) -> FpdfPage;
type FnClosePage = unsafe extern "C" fn(FpdfPage);
type FnGetPageWidthF = unsafe extern "C" fn(FpdfPage) -> f32;
type FnGetPageHeightF = unsafe extern "C" fn(FpdfPage) -> f32;
type FnBitmapCreateEx = unsafe extern "C" fn(c_int, c_int, c_int, *mut c_void, c_int) -> FpdfBitmap;
type FnBitmapFillRect =
    unsafe extern "C" fn(FpdfBitmap, c_int, c_int, c_int, c_int, c_ulong) -> c_int;
type FnRenderPageBitmap =
    unsafe extern "C" fn(FpdfBitmap, FpdfPage, c_int, c_int, c_int, c_int, c_int, c_int);
type FnBitmapGetBuffer = unsafe extern "C" fn(FpdfBitmap) -> *mut c_void;
type FnBitmapGetWidth = unsafe extern "C" fn(FpdfBitmap) -> c_int;
type FnBitmapGetHeight = unsafe extern "C" fn(FpdfBitmap) -> c_int;
type FnBitmapGetStride = unsafe extern "C" fn(FpdfBitmap) -> c_int;
type FnBitmapDestroy = unsafe extern "C" fn(FpdfBitmap);

/// FPDFBitmap_* format codes (fpdfview.h).
const FORMAT_GRAY: c_int = 1;
const FORMAT_BGRA: c_int = 4;

struct Api {
    init_library: FnInitLibrary,
    destroy_library: FnDestroyLibrary,
    load_mem_document: FnLoadMemDocument,
    get_last_error: FnGetLastError,
    get_page_count: FnGetPageCount,
    close_document: FnCloseDocument,
    load_page: FnLoadPage,
    close_page: FnClosePage,
    get_page_width_f: FnGetPageWidthF,
    get_page_height_f: FnGetPageHeightF,
    bitmap_create_ex: FnBitmapCreateEx,
    bitmap_fill_rect: FnBitmapFillRect,
    render_page_bitmap: FnRenderPageBitmap,
    bitmap_get_buffer: FnBitmapGetBuffer,
    bitmap_get_width: FnBitmapGetWidth,
    bitmap_get_height: FnBitmapGetHeight,
    bitmap_get_stride: FnBitmapGetStride,
    bitmap_destroy: FnBitmapDestroy,
}

/// Serialize every PDFium call: the library is not thread-safe.
fn call_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn init_refcount() -> &'static Mutex<usize> {
    static COUNT: OnceLock<Mutex<usize>> = OnceLock::new();
    COUNT.get_or_init(|| Mutex::new(0))
}

/// A loaded PDFium library. Cheap to clone (Arc inside documents);
/// the underlying library is initialised on first load and destroyed
/// when the last instance drops.
pub struct Pdfium {
    _lib: libloading::Library,
    api: Api,
    loaded_from: PathBuf,
}

impl Pdfium {
    /// Load PDFium from the standard search order:
    /// 1. `explicit` argument (file, or directory containing the file)
    /// 2. the `NATIVE_PRINT_PDFIUM_PATH` env var (same handling)
    /// 3. beside the current executable, then its `resources/` subdir
    ///    (the installed-app layout: `bundle.resources` in
    ///    tauri.conf.json drops `pdfium.dll` in the resource dir)
    /// 4. the bare library name (OS loader search: PATH / system dirs)
    pub fn load() -> Result<Arc<Self>, PdfiumError> {
        Self::load_from(None)
    }

    /// Like [`Pdfium::load`], with an explicit first candidate.
    pub fn load_from(explicit: Option<&Path>) -> Result<Arc<Self>, PdfiumError> {
        let candidates = candidate_paths(explicit);
        let mut last_load_error: Option<PdfiumError> = None;
        for cand in &candidates {
            match unsafe { libloading::Library::new(cand) } {
                Ok(lib) => {
                    let api = load_api(&lib)?;
                    {
                        let _guard = call_lock().lock().expect("pdfium lock poisoned");
                        let mut count = init_refcount().lock().expect("pdfium lock poisoned");
                        if *count == 0 {
                            unsafe { (api.init_library)() };
                        }
                        *count += 1;
                    }
                    return Ok(Arc::new(Self {
                        _lib: lib,
                        api,
                        loaded_from: cand.clone(),
                    }));
                }
                Err(e) => {
                    last_load_error = Some(PdfiumError::LoadFailed {
                        path: cand.display().to_string(),
                        source: e,
                    });
                }
            }
        }
        // If at least one candidate existed as a file but failed to
        // load, that is the more useful error; otherwise: not found.
        let any_file = candidates.iter().any(|c| c.is_file());
        if any_file {
            if let Some(e) = last_load_error {
                return Err(e);
            }
        }
        Err(PdfiumError::LibraryNotFound {
            searched: candidates.iter().map(|c| c.display().to_string()).collect(),
        })
    }

    /// Path the library was actually loaded from (diagnostics).
    pub fn loaded_from(&self) -> &Path {
        &self.loaded_from
    }

    /// Open a PDF held in memory. The bytes are copied into the
    /// document handle (PDFium copies what it needs at load time in
    /// the pinned build's `FPDF_LoadMemDocument` contract: the caller
    /// must keep `bytes` alive only for this call — we pass a buffer
    /// we own until the call returns; PDFium documents that the data
    /// must remain valid while the document is open, so this wrapper
    /// retains an owned copy inside [`PdfDocument`]).
    pub fn load_document(self: &Arc<Self>, bytes: &[u8]) -> Result<PdfDocument, PdfiumError> {
        let owned = bytes.to_vec();
        let _guard = call_lock().lock().expect("pdfium lock poisoned");
        let doc = unsafe {
            (self.api.load_mem_document)(owned.as_ptr(), owned.len() as c_int, std::ptr::null())
        };
        if doc.is_null() {
            let code = unsafe { (self.api.get_last_error)() } as u32;
            return Err(PdfiumError::InvalidDocument {
                code,
                detail: last_error_detail(code),
            });
        }
        Ok(PdfDocument {
            pdfium: Arc::clone(self),
            doc,
            _bytes: owned,
        })
    }
}

impl Drop for Pdfium {
    fn drop(&mut self) {
        let _guard = call_lock().lock().expect("pdfium lock poisoned");
        let mut count = init_refcount().lock().expect("pdfium lock poisoned");
        *count = count.saturating_sub(1);
        if *count == 0 {
            unsafe { (self.api.destroy_library)() };
        }
    }
}

fn last_error_detail(code: u32) -> String {
    // FPDF_ERR_* codes from fpdfview.h.
    match code {
        0 => "success (document handle was null anyway)".into(),
        1 => "unknown error".into(),
        2 => "file not found or could not be opened".into(),
        3 => "file format is not PDF".into(),
        4 => "invalid password".into(),
        5 => "unsupported security scheme".into(),
        6 => "page not found or content error".into(),
        _ => format!("unrecognised error code {code}"),
    }
}

/// Every candidate location, in search order. Exposed for diagnostics
/// and tests.
pub fn candidate_paths(explicit: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |base: &Path| {
        if base.is_file() {
            out.push(base.to_path_buf());
        } else {
            for name in LIB_NAMES {
                out.push(base.join(name));
            }
        }
    };
    if let Some(p) = explicit {
        push(p);
    }
    if let Ok(env) = std::env::var(ENV_PDFIUM_PATH) {
        if !env.is_empty() {
            push(Path::new(&env));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in LIB_NAMES {
                out.push(dir.join(name));
                out.push(dir.join("resources").join(name));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for name in LIB_NAMES {
            out.push(cwd.join(name));
            out.push(cwd.join("resources").join(name));
        }
    }
    for name in LIB_NAMES {
        out.push(PathBuf::from(name));
    }
    out
}

fn load_api(lib: &libloading::Library) -> Result<Api, PdfiumError> {
    macro_rules! sym {
        ($name:expr, $t:ty) => {
            *unsafe { lib.get::<$t>($name.as_bytes()) }
                .map_err(|_| PdfiumError::MissingSymbol($name.into()))?
        };
    }
    Ok(Api {
        init_library: sym!("FPDF_InitLibrary", FnInitLibrary),
        destroy_library: sym!("FPDF_DestroyLibrary", FnDestroyLibrary),
        load_mem_document: sym!("FPDF_LoadMemDocument", FnLoadMemDocument),
        get_last_error: sym!("FPDF_GetLastError", FnGetLastError),
        get_page_count: sym!("FPDF_GetPageCount", FnGetPageCount),
        close_document: sym!("FPDF_CloseDocument", FnCloseDocument),
        load_page: sym!("FPDF_LoadPage", FnLoadPage),
        close_page: sym!("FPDF_ClosePage", FnClosePage),
        get_page_width_f: sym!("FPDF_GetPageWidthF", FnGetPageWidthF),
        get_page_height_f: sym!("FPDF_GetPageHeightF", FnGetPageHeightF),
        bitmap_create_ex: sym!("FPDFBitmap_CreateEx", FnBitmapCreateEx),
        bitmap_fill_rect: sym!("FPDFBitmap_FillRect", FnBitmapFillRect),
        render_page_bitmap: sym!("FPDF_RenderPageBitmap", FnRenderPageBitmap),
        bitmap_get_buffer: sym!("FPDFBitmap_GetBuffer", FnBitmapGetBuffer),
        bitmap_get_width: sym!("FPDFBitmap_GetWidth", FnBitmapGetWidth),
        bitmap_get_height: sym!("FPDFBitmap_GetHeight", FnBitmapGetHeight),
        bitmap_get_stride: sym!("FPDFBitmap_GetStride", FnBitmapGetStride),
        bitmap_destroy: sym!("FPDFBitmap_Destroy", FnBitmapDestroy),
    })
}

/// An open PDF document. Rendering is serialized internally; the
/// document may be shared, but renders still run one at a time.
pub struct PdfDocument {
    pdfium: Arc<Pdfium>,
    doc: FpdfDocument,
    // PDFium requires the source buffer to outlive the document.
    _bytes: Vec<u8>,
}

// The raw handle is only ever touched under the process-wide call
// lock, so sharing the document across threads cannot race PDFium.
unsafe impl Send for PdfDocument {}
unsafe impl Sync for PdfDocument {}

impl PdfDocument {
    pub fn page_count(&self) -> usize {
        let _guard = call_lock().lock().expect("pdfium lock poisoned");
        let n = unsafe { (self.pdfium.api.get_page_count)(self.doc) };
        n.max(0) as usize
    }

    /// Page size in millimetres (from PDF points, 1pt = 1/72in).
    pub fn page_size_mm(&self, index: usize) -> Result<PageSize, PdfiumError> {
        let _guard = call_lock().lock().expect("pdfium lock poisoned");
        let page = self.load_page_locked(index)?;
        let w_pt = unsafe { (self.pdfium.api.get_page_width_f)(page) } as f64;
        let h_pt = unsafe { (self.pdfium.api.get_page_height_f)(page) } as f64;
        unsafe { (self.pdfium.api.close_page)(page) };
        Ok(PageSize::new(w_pt * 25.4 / 72.0, h_pt * 25.4 / 72.0))
    }

    fn load_page_locked(&self, index: usize) -> Result<FpdfPage, PdfiumError> {
        let count = unsafe { (self.pdfium.api.get_page_count)(self.doc) };
        if index as c_int >= count {
            return Err(PdfiumError::PageOutOfRange(index));
        }
        let page = unsafe { (self.pdfium.api.load_page)(self.doc, index as c_int) };
        if page.is_null() {
            return Err(PdfiumError::PageOutOfRange(index));
        }
        Ok(page)
    }

    /// Render one page following the shared render plan: the plan
    /// picks the DPI (highest at or under the request that fits), and
    /// this function additionally refuses any allocation over the
    /// per-page ceiling — including the grayscale staging buffer a
    /// Threshold render needs — instead of trusting the plan alone.
    ///
    /// Pixel layout of the result, by mode:
    /// - Color: BGRA, 4 bytes/px, rows packed (no padding)
    /// - Grayscale: 1 byte/px (0 black .. 255 white)
    /// - Threshold: 1-bit packed MSB-first via MonoBitmap's pad policy
    pub fn render_page(
        &self,
        index: usize,
        mode: RenderMode,
        requested_dpi: u32,
    ) -> Result<RenderedPage, PdfiumError> {
        let _guard = call_lock().lock().expect("pdfium lock poisoned");
        let api = &self.pdfium.api;
        let page = self.load_page_locked(index)?;
        let result = self.render_locked(api, page, mode, requested_dpi);
        unsafe { (api.close_page)(page) };
        result
    }

    fn render_locked(
        &self,
        api: &Api,
        page: FpdfPage,
        mode: RenderMode,
        requested_dpi: u32,
    ) -> Result<RenderedPage, PdfiumError> {
        let w_pt = unsafe { (api.get_page_width_f)(page) } as f64;
        let h_pt = unsafe { (api.get_page_height_f)(page) } as f64;
        let size = PageSize::new(w_pt * 25.4 / 72.0, h_pt * 25.4 / 72.0);
        let plan = plan_render(size, requested_dpi, mode);
        let (w, h) = (plan.width_px, plan.height_px);
        // Bytes the render itself will allocate: color/gray are the
        // final buffers; a threshold render stages through grayscale.
        let staging_bpp: u64 = match mode {
            RenderMode::Color => 4,
            RenderMode::Grayscale | RenderMode::Threshold => 1,
        };
        let staging = (w as u64) * (h as u64) * staging_bpp;
        let needed = staging.max(plan.bytes_estimate);
        if needed > PER_PAGE_CEILING_BYTES || plan.bytes_estimate > PER_PAGE_CEILING_BYTES {
            return Err(PdfiumError::CeilingExceeded {
                needed,
                ceiling: PER_PAGE_CEILING_BYTES,
            });
        }
        let format = match mode {
            RenderMode::Color => FORMAT_BGRA,
            _ => FORMAT_GRAY,
        };
        let bitmap = unsafe {
            (api.bitmap_create_ex)(w as c_int, h as c_int, format, std::ptr::null_mut(), 0)
        };
        if bitmap.is_null() {
            return Err(PdfiumError::BitmapFailed(w, h));
        }
        let rendered = unsafe {
            // White background (BGRA 0xFFFFFFFF) so transparent page
            // areas print as paper, not black.
            (api.bitmap_fill_rect)(bitmap, 0, 0, w as c_int, h as c_int, 0xFFFF_FFFF);
            (api.render_page_bitmap)(bitmap, page, 0, 0, w as c_int, h as c_int, 0, 0);
            let bw = (api.bitmap_get_width)(bitmap) as u32;
            let bh = (api.bitmap_get_height)(bitmap) as u32;
            let stride = (api.bitmap_get_stride)(bitmap) as usize;
            let buf = (api.bitmap_get_buffer)(bitmap) as *const u8;
            if buf.is_null() || bw != w || bh != h {
                (api.bitmap_destroy)(bitmap);
                return Err(PdfiumError::BitmapFailed(w, h));
            }
            let row_bytes = (bw as usize) * (staging_bpp as usize);
            let mut pixels = vec![0u8; row_bytes * bh as usize];
            for row in 0..bh as usize {
                let src = buf.add(row * stride);
                let dst = &mut pixels[row * row_bytes..(row + 1) * row_bytes];
                std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), row_bytes);
            }
            (api.bitmap_destroy)(bitmap);
            pixels
        };
        let pixels = match mode {
            RenderMode::Threshold => {
                let mono = print_raw::MonoBitmap::from_grayscale(&rendered, w, h, 128)
                    .expect("grayscale buffer length matches width*height by construction");
                mono.data
            }
            _ => rendered,
        };
        Ok(RenderedPage {
            width_px: w,
            height_px: h,
            mode,
            dpi: plan.dpi,
            pixels,
        })
    }
}

impl Drop for PdfDocument {
    fn drop(&mut self) {
        let _guard = call_lock().lock().expect("pdfium lock poisoned");
        unsafe { (self.pdfium.api.close_document)(self.doc) };
    }
}

/// One rendered page, pixels packed row-major in the layout described
/// on [`PdfDocument::render_page`].
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedPage {
    pub width_px: u32,
    pub height_px: u32,
    pub mode: RenderMode,
    pub dpi: u32,
    pub pixels: Vec<u8>,
}

impl RenderedPage {
    /// Fraction of pixels that are not background (white). Color and
    /// grayscale compare luminance; threshold counts black dots.
    /// Useful for blank-page detection and render sanity checks.
    pub fn non_background_fraction(&self) -> f64 {
        if self.pixels.is_empty() {
            return 0.0;
        }
        match self.mode {
            RenderMode::Color => {
                let total = (self.width_px as usize) * (self.height_px as usize);
                let dark = self
                    .pixels
                    .chunks_exact(4)
                    .filter(|px| px[0] < 250 || px[1] < 250 || px[2] < 250)
                    .count();
                dark as f64 / total.max(1) as f64
            }
            RenderMode::Grayscale => {
                let dark = self.pixels.iter().filter(|&&p| p < 250).count();
                dark as f64 / self.pixels.len() as f64
            }
            RenderMode::Threshold => {
                let total = (self.width_px as u64) * (self.height_px as u64);
                let dark: u64 = self.pixels.iter().map(|b| b.count_ones() as u64).sum();
                dark as f64 / total.max(1) as f64
            }
        }
    }

    pub fn is_blank(&self) -> bool {
        self.non_background_fraction() == 0.0
    }
}

// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Restrict Windows DLL search to safe default directories (application dir,
/// System32, and explicit AddDllDirectory entries), excluding CWD and PATH
/// so pdfium.dll and delay-loaded DLLs cannot be hijacked from the working dir.
#[cfg(windows)]
fn harden_dll_search_path() {
    const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: u32 = 0x0000_1000;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetDefaultDllDirectories(directory_flags: u32) -> i32;
    }
    // SAFETY: Process-wide loader flag set once at startup before any threads or DLL loads.
    unsafe {
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    }
}

fn main() {
    #[cfg(windows)]
    harden_dll_search_path();
    odoo_native_print_app::run();
}

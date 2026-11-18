//! npwin_probe: command-line probe for the Windows printing layer.
//! Used by CI (windows-print-smoke job) and by the printer-lab workflow
//! on a self-hosted runner with real printers attached.
//!
//! Usage:
//!   npwin_probe list                      discover printers + statuses
//!   npwin_probe status <printer>          sensor read for one printer
//!   npwin_probe forms <printer>           paper forms of one printer
//!   npwin_probe escpos-test <printer>     print the 80mm calibration
//!                                         page (a REAL print)
//!   npwin_probe pdf-gdi <printer> <file.pdf>
//!                                         print a PDF via the GDI
//!                                         bitmap backend (spike S5;
//!                                         use "Microsoft Print to PDF"
//!                                         as the printer for proof)
//!   npwin_probe pdf-gdi-test <printer>    GDI-print the built-in
//!                                         calibration PDF (a REAL
//!                                         print; for the lab / S5)
//!   npwin_probe raw <printer> <file>      send a file's bytes RAW
//! Exit codes: 0 ok, 2 usage, 3 platform/FFI error.

use std::process::ExitCode;

fn fail<E: std::fmt::Display>(e: E) -> ExitCode {
    eprintln!("error: {e}");
    ExitCode::from(3)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("list") => match print_win::discover_printers() {
            Ok(printers) => {
                println!("printers.count={}", printers.len());
                for p in &printers {
                    println!(
                        "printer name={:?} status={:?} default={} kind={:?} port={:?} driver={:?}",
                        p.name, p.status, p.is_default, p.kind, p.port, p.driver
                    );
                }
                match print_win::default_printer_name() {
                    Ok(name) => println!("default={name:?}"),
                    Err(e) => println!("default.error={e}"),
                }
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        Some("status") => match args.get(2) {
            Some(name) => match print_win::printer_status(name) {
                Ok(s) => {
                    println!("status {name:?} = {s:?}");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            },
            None => ExitCode::from(2),
        },
        Some("forms") => match args.get(2) {
            Some(name) => match print_win::paper_forms(name) {
                Ok(forms) => {
                    println!("forms.count={}", forms.len());
                    for f in &forms {
                        println!(
                            "form name={:?} size={:.1}x{:.1}mm roll={}",
                            f.name, f.size.width_mm, f.size.height_mm, f.roll
                        );
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            },
            None => ExitCode::from(2),
        },
        Some("escpos-test") => match args.get(2) {
            Some(name) => {
                let bm = print_render::calibration::calibration_thermal_80mm();
                let stream = print_raw::EscPos::new()
                    .raster(&bm)
                    .feed_lines(4)
                    .cut_partial();
                match print_win::print_raw(name, "Native Print calibration", stream.bytes()) {
                    Ok(id) => {
                        println!("escpos-test spooled job_id={id} printer={name:?}");
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e),
                }
            }
            None => ExitCode::from(2),
        },
        Some("pdf-gdi-test") => match args.get(2) {
            Some(name) => {
                let bytes = print_render::calibration::calibration_pdf();
                match print_win::print_pdf_gdi(
                    name,
                    "Native Print calibration (GDI)",
                    &bytes,
                    print_render::RenderMode::Grayscale,
                    300,
                ) {
                    Ok(pages) => {
                        println!(
                            "pdf-gdi-test printed pages={pages} bytes={} printer={name:?}",
                            bytes.len()
                        );
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e),
                }
            }
            None => ExitCode::from(2),
        },
        Some("pdf-gdi") => match (args.get(2), args.get(3)) {
            (Some(name), Some(path)) => match std::fs::read(path) {
                Ok(bytes) => match print_win::print_pdf_gdi(
                    name,
                    "Native Print PDF (GDI)",
                    &bytes,
                    print_render::RenderMode::Grayscale,
                    300,
                ) {
                    Ok(pages) => {
                        println!(
                            "pdf-gdi printed pages={pages} bytes={} printer={name:?} file={path:?}",
                            bytes.len()
                        );
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e),
                },
                Err(e) => fail(e),
            },
            _ => ExitCode::from(2),
        },
        Some("raw") => match (args.get(2), args.get(3)) {
            (Some(name), Some(path)) => match std::fs::read(path) {
                Ok(bytes) => match print_win::print_raw(name, "Native Print raw job", &bytes) {
                    Ok(id) => {
                        println!(
                            "raw spooled job_id={id} bytes={} printer={name:?}",
                            bytes.len()
                        );
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e),
                },
                Err(e) => fail(e),
            },
            _ => ExitCode::from(2),
        },
        _ => {
            eprintln!(
                "usage: npwin_probe <list|status|forms|escpos-test|pdf-gdi|pdf-gdi-test|raw> ..."
            );
            ExitCode::from(2)
        }
    }
}

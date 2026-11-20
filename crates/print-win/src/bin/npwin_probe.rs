//! npwin_probe: command-line probe for the Windows printing layer.
//! Used by CI (windows-print-smoke job) and by the printer-lab workflow
//! on a self-hosted runner with real printers attached.
//!
//! Usage:
//!   npwin_probe [--json] list                      discover printers + statuses
//!   npwin_probe [--json] status <printer>          sensor read for one printer
//!   npwin_probe [--json] forms <printer>           paper forms of one printer
//!   npwin_probe [--json] escpos-test <printer>     print the 80mm calibration
//!                                                  page (a REAL print)
//!   npwin_probe [--json] pdf-gdi <printer> <file.pdf>
//!                                                  print a PDF via the GDI
//!                                                  bitmap backend (spike S5;
//!                                                  use "Microsoft Print to PDF"
//!                                                  as the printer for proof)
//!   npwin_probe [--json] pdf-gdi-test <printer>    GDI-print the built-in
//!                                                  calibration PDF (a REAL
//!                                                  print; for the lab / S5)
//!   npwin_probe [--json] raw <printer> <file>      send a file's bytes RAW
//! Exit codes: 0 ok, 2 usage, 3 platform/FFI error.

use std::fmt::Write;
use std::process::ExitCode;

fn parse_args(args: &[String]) -> (bool, Vec<&str>) {
    let mut json = false;
    let mut pos = Vec::new();
    for arg in args.iter().skip(1) {
        if arg == "--json" {
            json = true;
        } else {
            pos.push(arg.as_str());
        }
    }
    (json, pos)
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_f64(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.1}")
    } else {
        "null".to_string()
    }
}

fn json_opt_str(v: Option<&str>) -> String {
    match v {
        Some(s) => json_str(s),
        None => "null".to_string(),
    }
}

fn format_list_json(
    printers: &[print_core::PrinterInfo],
    default_name: Option<&str>,
    default_error: Option<&str>,
) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "{{\"ok\":true,\"command\":\"list\",\"count\":{},\"default\":{},\"default_error\":{},\"printers\":[",
        printers.len(),
        json_opt_str(default_name),
        json_opt_str(default_error)
    );
    for (idx, p) in printers.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let status = format!("{:?}", p.status);
        let kind = format!("{:?}", p.kind);
        let _ = write!(
            out,
            "{{\"name\":{},\"status\":{},\"is_default\":{},\"kind\":{},\"protocol\":{},\"printer_class\":{},\"port\":{},\"driver\":{},\"stable_id\":{}}}",
            json_str(&p.name),
            json_str(&status),
            p.is_default,
            json_str(&kind),
            json_str(&p.protocol),
            json_str(&p.printer_class),
            json_str(&p.port),
            json_str(&p.driver),
            json_str(&p.stable_id)
        );
    }
    out.push_str("]}");
    out
}

fn format_health_json(printer: &str, health: &print_core::PrinterHealth) -> String {
    let mut out = String::new();
    let status = format!("{:?}", health.primary_status());
    let source = format!("{:?}", health.source);
    let _ = write!(
        out,
        "{{\"ok\":true,\"command\":\"status\",\"printer\":{},\"status\":{},\"blocks_printing\":{},\"source\":{},\"observed_at\":{},\"raw\":{},\"conditions\":[",
        json_str(printer),
        json_str(&status),
        health.blocks_printing(),
        json_str(&source),
        health.observed_at,
        json_opt_str(health.raw.as_deref())
    );
    for (idx, cond) in health.conditions.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let label = format!("{cond:?}");
        out.push_str(&json_str(&label));
    }
    out.push_str("]}");
    out
}

fn format_forms_json(printer: &str, forms: &[print_core::Form]) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "{{\"ok\":true,\"command\":\"forms\",\"printer\":{},\"count\":{},\"forms\":[",
        json_str(printer),
        forms.len()
    );
    for (idx, f) in forms.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let paper_id = match f.paper_id {
            Some(id) => id.to_string(),
            None => "null".to_string(),
        };
        let _ = write!(
            out,
            "{{\"name\":{},\"width_mm\":{},\"height_mm\":{},\"roll\":{},\"paper_id\":{}}}",
            json_str(&f.name),
            json_f64(f.size.width_mm),
            json_f64(f.size.height_mm),
            f.roll,
            paper_id
        );
    }
    out.push_str("]}");
    out
}

fn fail<E: std::fmt::Display>(e: E, json: bool) -> ExitCode {
    if json {
        println!("{{\"ok\":false,\"error\":{}}}", json_str(&e.to_string()));
    } else {
        eprintln!("error: {e}");
    }
    ExitCode::from(3)
}

fn missing_arg(json: bool) -> ExitCode {
    if json {
        println!("{{\"ok\":false,\"error\":\"missing required argument\"}}");
    }
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (json, pos) = parse_args(&args);
    match pos.first().copied() {
        Some("list") => match print_win::discover_printers() {
            Ok(printers) => {
                let default_res = print_win::default_printer_name();
                if json {
                    let (def_name, def_err) = match &default_res {
                        Ok(name) => (Some(name.as_str()), None),
                        Err(e) => (None, Some(e.to_string())),
                    };
                    println!(
                        "{}",
                        format_list_json(&printers, def_name, def_err.as_deref())
                    );
                } else {
                    println!("printers.count={}", printers.len());
                    for p in &printers {
                        println!(
                            "printer name={:?} status={:?} default={} kind={:?} port={:?} driver={:?}",
                            p.name, p.status, p.is_default, p.kind, p.port, p.driver
                        );
                    }
                    match default_res {
                        Ok(name) => println!("default={name:?}"),
                        Err(e) => println!("default.error={e}"),
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => fail(e, json),
        },
        Some("status") => match pos.get(1).copied() {
            Some(name) => {
                if json {
                    match print_win::printer_health(name) {
                        Ok(h) => {
                            println!("{}", format_health_json(name, &h));
                            ExitCode::SUCCESS
                        }
                        Err(e) => fail(e, json),
                    }
                } else {
                    match print_win::printer_status(name) {
                        Ok(s) => {
                            println!("status {name:?} = {s:?}");
                            ExitCode::SUCCESS
                        }
                        Err(e) => fail(e, json),
                    }
                }
            }
            None => missing_arg(json),
        },
        Some("forms") => match pos.get(1).copied() {
            Some(name) => match print_win::paper_forms(name) {
                Ok(forms) => {
                    if json {
                        println!("{}", format_forms_json(name, &forms));
                    } else {
                        println!("forms.count={}", forms.len());
                        for f in &forms {
                            println!(
                                "form name={:?} size={:.1}x{:.1}mm roll={}",
                                f.name, f.size.width_mm, f.size.height_mm, f.roll
                            );
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e, json),
            },
            None => missing_arg(json),
        },
        Some("escpos-test") => match pos.get(1).copied() {
            Some(name) => {
                let bm = print_render::calibration::calibration_thermal_80mm();
                let stream = print_raw::EscPos::new()
                    .raster(&bm)
                    .feed_lines(4)
                    .cut_partial();
                match print_win::print_raw(name, "Native Print calibration", stream.bytes()) {
                    Ok(id) => {
                        if json {
                            println!(
                                "{{\"ok\":true,\"command\":\"escpos-test\",\"printer\":{},\"job_id\":{id}}}",
                                json_str(name)
                            );
                        } else {
                            println!("escpos-test spooled job_id={id} printer={name:?}");
                        }
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e, json),
                }
            }
            None => missing_arg(json),
        },
        Some("pdf-gdi-test") => match pos.get(1).copied() {
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
                        if json {
                            println!(
                                "{{\"ok\":true,\"command\":\"pdf-gdi-test\",\"printer\":{},\"pages\":{pages},\"bytes\":{}}}",
                                json_str(name),
                                bytes.len()
                            );
                        } else {
                            println!(
                                "pdf-gdi-test printed pages={pages} bytes={} printer={name:?}",
                                bytes.len()
                            );
                        }
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e, json),
                }
            }
            None => missing_arg(json),
        },
        Some("pdf-gdi") => match (pos.get(1).copied(), pos.get(2).copied()) {
            (Some(name), Some(path)) => match std::fs::read(path) {
                Ok(bytes) => match print_win::print_pdf_gdi(
                    name,
                    "Native Print PDF (GDI)",
                    &bytes,
                    print_render::RenderMode::Grayscale,
                    300,
                ) {
                    Ok(pages) => {
                        if json {
                            println!(
                                "{{\"ok\":true,\"command\":\"pdf-gdi\",\"printer\":{},\"file\":{},\"pages\":{pages},\"bytes\":{}}}",
                                json_str(name),
                                json_str(path),
                                bytes.len()
                            );
                        } else {
                            println!(
                                "pdf-gdi printed pages={pages} bytes={} printer={name:?} file={path:?}",
                                bytes.len()
                            );
                        }
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e, json),
                },
                Err(e) => fail(e, json),
            },
            _ => missing_arg(json),
        },
        Some("raw") => match (pos.get(1).copied(), pos.get(2).copied()) {
            (Some(name), Some(path)) => match std::fs::read(path) {
                Ok(bytes) => match print_win::print_raw(name, "Native Print raw job", &bytes) {
                    Ok(id) => {
                        if json {
                            println!(
                                "{{\"ok\":true,\"command\":\"raw\",\"printer\":{},\"file\":{},\"job_id\":{id},\"bytes\":{}}}",
                                json_str(name),
                                json_str(path),
                                bytes.len()
                            );
                        } else {
                            println!(
                                "raw spooled job_id={id} bytes={} printer={name:?}",
                                bytes.len()
                            );
                        }
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e, json),
                },
                Err(e) => fail(e, json),
            },
            _ => missing_arg(json),
        },
        _ => {
            let usage =
                "usage: npwin_probe [--json] <list|status|forms|escpos-test|pdf-gdi|pdf-gdi-test|raw> ...";
            if json {
                println!("{{\"ok\":false,\"error\":{}}}", json_str(usage));
            } else {
                eprintln!("{usage}");
            }
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_accepts_json_flag_anywhere() {
        let a: Vec<String> = ["npwin_probe", "--json", "status", "EPSON TM-T88V"]
            .into_iter()
            .map(String::from)
            .collect();
        let (json, pos) = parse_args(&a);
        assert!(json);
        assert_eq!(pos, vec!["status", "EPSON TM-T88V"]);

        let b: Vec<String> = ["npwin_probe", "forms", "HP LaserJet", "--json"]
            .into_iter()
            .map(String::from)
            .collect();
        let (json_b, pos_b) = parse_args(&b);
        assert!(json_b);
        assert_eq!(pos_b, vec!["forms", "HP LaserJet"]);

        let c: Vec<String> = ["npwin_probe", "list"]
            .into_iter()
            .map(String::from)
            .collect();
        let (json_c, pos_c) = parse_args(&c);
        assert!(!json_c);
        assert_eq!(pos_c, vec!["list"]);
    }

    #[test]
    fn json_escaping_handles_unc_paths_quotes_and_controls() {
        assert_eq!(
            json_str(r#"\\server\"share"\n"#),
            r#""\\\\server\\\"share\"\\n""#
        );
        assert_eq!(json_str("a\nb\tc\x01"), r#""a\nb\tc\u0001""#);
        assert_eq!(json_f64(210.0), "210.0");
        assert_eq!(json_f64(f64::NAN), "null");
    }

    #[test]
    fn format_forms_json_emits_structured_records() {
        let forms = vec![
            print_core::Form::sheet("A4", print_core::PageSize::new(210.0, 297.0)).with_id(9),
            print_core::Form::roll("80mm Roll", 80.0),
        ];
        let out = format_forms_json(r"\\pos\receipt", &forms);
        assert!(out.contains(r#""ok":true,"command":"forms""#));
        assert!(out.contains(r#""printer":"\\\\pos\\receipt""#));
        assert!(out.contains(r#""count":2"#));
        assert!(out.contains(
            r#""name":"A4","width_mm":210.0,"height_mm":297.0,"roll":false,"paper_id":9"#
        ));
        assert!(out.contains(
            r#""name":"80mm Roll","width_mm":80.0,"height_mm":0.0,"roll":true,"paper_id":null"#
        ));
    }
}

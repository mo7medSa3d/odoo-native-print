#![no_main]
// Fuzz target for the PDF inspection parser (spec section 7: PDFs are
// hostile input). Run on Linux: cargo install cargo-fuzz, then
// cargo +nightly fuzz run pdf_info -- -max_total_time=60
// The parser must never panic and never loop on arbitrary bytes; any
// Ok/Err result is acceptable.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = print_render::parse_pdf_info(data);
});

#![no_main]
// Fuzz target for the IPP response parser: printers and print servers
// are network input, so their response bytes are hostile input too.
// Run on Linux: cargo install cargo-fuzz, then
// cargo +nightly fuzz run ipp_response -- -max_total_time=60
// The parser must never panic and never loop on arbitrary bytes; any
// Ok/Err result is acceptable.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = print_ipp::parse_message(data);
});

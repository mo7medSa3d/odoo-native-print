#![no_main]
// Fuzz target for the PNG decoder: POS receipt images arrive from the
// server/POS page and are decoded before rasterization, so the bytes
// are hostile input. Run on Linux: cargo install cargo-fuzz, then
// cargo +nightly fuzz run png_decode -- -max_total_time=60
// The decoder must never panic and never loop on arbitrary bytes; any
// Ok/Err result is acceptable.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = print_render::decode_png(data);
});

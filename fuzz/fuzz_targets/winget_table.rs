//! `winget list` output is localised, width-formatted, and full of \r progress spinners.
#![no_main]
use libfuzzer_sys::fuzz_target;
use linuxify::scan::programs::parse_winget_table;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = parse_winget_table(&text);
});

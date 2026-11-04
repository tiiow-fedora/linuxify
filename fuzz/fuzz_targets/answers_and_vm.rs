//! The --answers file for `linuxify choose`, and small CLI value parsers.
#![no_main]
use libfuzzer_sys::fuzz_target;
use linuxify::chooser::{validate_answers, Answers};
use linuxify::vm::Backend;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(a) = serde_json::from_str::<Answers>(&text) {
        let _ = validate_answers(&a);
    }
    let _ = Backend::parse(&text);
});

//! Checksum-file parsing and https host extraction (ISO download trust decisions).
#![no_main]
use libfuzzer_sys::fuzz_target;
use linuxify::iso::{host_of, parse_checksums};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for (name, hash) in parse_checksums(&text) {
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(!name.is_empty() && !name.contains('/') && !name.contains('\\'), "file name is a path");
    }
    if let Some(h) = host_of(&text) {
        assert!(text.starts_with("https://"));
        assert!(!h.is_empty() && !h.contains(['@', '/', '?', '#', ':']));
    }
});

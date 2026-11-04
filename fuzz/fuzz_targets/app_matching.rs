//! Program-name normalisation and classification against the shipped app map.
#![no_main]
use libfuzzer_sys::fuzz_target;
use linuxify::mapping::{classify, pattern_match_len, AppMap, EMBEDDED_APP_MAP};
use linuxify::scan::programs::{normalize_name, steam_appid_from_key, Program};
use std::sync::OnceLock;

static MAP: OnceLock<AppMap> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data).into_owned();
    let n = normalize_name(&s);
    assert_eq!(n, normalize_name(&n), "normalize_name must be idempotent");
    assert!(!n.starts_with(' ') && !n.ends_with(' ') && !n.contains("  "));
    let _ = steam_appid_from_key(&s);
    let split = s.len() / 2;
    let (pat, hay) = s.split_at(s.floor_char_boundary(split));
    let _ = pattern_match_len(pat, hay);
    let map = MAP.get_or_init(|| AppMap::parse(EMBEDDED_APP_MAP).expect("shipped map parses"));
    let p = Program { name: s, publisher: None, version: None, sources: vec![], winget_id: None, steam_appid: None, microsoft_msix: false };
    let _ = classify(&p, map);
});

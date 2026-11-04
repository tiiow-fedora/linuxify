//! JSON state Linuxify writes itself and later trusts: install-state.json (drives --undo),
//! the migration manifest and progress log, the saved scan, bcdedit output.
#![no_main]
use libfuzzer_sys::fuzz_target;
use linuxify::install::{parse_bcd_guid, InstallState};
use linuxify::migrate::{parse_progress, path_within, Manifest};
use linuxify::scan::ScanReport;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = serde_json::from_str::<InstallState>(&text);
    let _ = serde_json::from_str::<Manifest>(&text);
    let _ = serde_json::from_str::<ScanReport>(&text);
    let _ = parse_progress(&text);
    if let Some(g) = parse_bcd_guid(&text) {
        assert_eq!(g.len(), 38);
        assert!(g.starts_with('{') && g.ends_with('}'));
    }
    let (a, b) = text.split_at(text.floor_char_boundary(text.len() / 2));
    let (a, b) = (std::path::Path::new(a), std::path::Path::new(b));
    assert!(path_within(a, a));
    let _ = path_within(a, b);
});

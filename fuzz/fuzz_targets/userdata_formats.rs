//! Files read from the user's profile: Mozilla profiles.ini, Steam .vdf/.acf, Chromium Local State.
#![no_main]
use libfuzzer_sys::fuzz_target;
use linuxify::scan::userdata::*;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for p in parse_profiles_ini(&text) {
        assert!(!p.path.is_empty());
    }
    let _ = parse_library_folders(&text);
    let _ = parse_app_manifest(&text);
    let _ = parse_chromium_local_state(&text);
});

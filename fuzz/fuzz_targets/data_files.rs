//! The three JSON data files users may override with --data-dir: distros.json, isos.json,
//! app_map.json. Accepting must imply the documented invariants; nothing may panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use linuxify::{chooser::DistroList, iso::IsoList, mapping::AppMap};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };

    if let Ok(list) = DistroList::parse(text) {
        let mut ids = std::collections::HashSet::new();
        for d in &list.distros {
            assert!(ids.insert(d.id.clone()), "accepted duplicate id");
            assert!(d.homepage.starts_with("https://"));
            assert!(d.traits.values().all(|v| *v <= 5));
        }
    }

    if let Ok(list) = IsoList::parse(text) {
        for s in &list.sources {
            let host = linuxify::iso::host_of(&s.checksum_url).expect("accepted a non-https checksum_url");
            assert!(s.allowed_hosts.iter().any(|h| h == host), "checksum host not in allowed_hosts");
            if let Some(b) = &s.download_base {
                assert!(b.starts_with("https://") && b.ends_with('/'));
            }
        }
    }

    if let Ok(map) = AppMap::parse(text) {
        assert!(map.validate().is_empty());
        let _ = map.lint();
    }
});

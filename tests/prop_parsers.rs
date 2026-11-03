//! Property tests for every parser and config reader. They check two things for arbitrary input:
//! nothing panics, and anything a parser accepts satisfies the invariants the rest of the program
//! (and the safety model in docs/safety-model.md) relies on. The cargo-fuzz targets in `fuzz/`
//! cover the same functions with coverage-guided input.

use linuxify::chooser::{validate_answers, Answers, DistroList, QUESTIONS};
use linuxify::fwboot::{fix_load_option, load_option_description, parse_gpt_entries, GptEntry};
use linuxify::install::parse_bcd_guid;
use linuxify::iso::{host_of as iso_host_of, parse_checksums, IsoList};
use linuxify::mapping::{classify, pattern_match_len, AppMap, EMBEDDED_APP_MAP};
use linuxify::migrate::{parse_progress, path_within};
use linuxify::scan::programs::{normalize_name, parse_winget_table, steam_appid_from_key, Program};
use linuxify::scan::userdata::{parse_app_manifest, parse_chromium_local_state, parse_library_folders, parse_profiles_ini};
use linuxify::webquiz::*;
use proptest::prelude::*;
use std::path::Path;

fn cfg() -> ProptestConfig {
    ProptestConfig { cases: 512, ..ProptestConfig::default() }
}

/// Text that looks vaguely like the real thing more often than pure noise does.
fn texty() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[ -~\\n\\r\\t\"=\\[\\]{}:,#%/\\\\-]{0,300}",
        prop::collection::vec(prop_oneof![Just("\"path\""), Just("\"name\""), Just("["), Just("Profile0"), Just("\n"), Just(" "), Just("="), Just("{"), Just("}"), Just("\r"), Just("-----"), Just("Name  Id  Version")], 0..40)
            .prop_map(|v| v.concat()),
    ]
}

proptest! {
    #![proptest_config(cfg())]

    // ---- data files ------------------------------------------------------------------------
    #[test]
    fn distros_parse_never_panics_and_accepts_only_valid(s in texty()) {
        if let Ok(list) = DistroList::parse(&s) {
            let mut ids = std::collections::HashSet::new();
            for d in &list.distros {
                prop_assert!(ids.insert(d.id.clone()));
                prop_assert!(d.homepage.starts_with("https://"));
                prop_assert!(d.traits.values().all(|v| *v <= 5));
            }
        }
    }

    #[test]
    fn isos_parse_never_panics(s in texty()) {
        let _ = IsoList::parse(&s);
    }

    #[test]
    fn app_map_parse_never_panics(s in texty()) {
        if let Ok(m) = AppMap::parse(&s) {
            prop_assert!(m.validate().is_empty());
        }
    }

    #[test]
    fn mutated_shipped_data_never_panics(cut in 0usize..200_000, junk in "[ -~]{0,8}") {
        // Truncations and small insertions of the real files exercise deeper code than random text.
        for src in [linuxify::chooser::EMBEDDED_DISTROS, linuxify::iso::EMBEDDED_ISOS, EMBEDDED_APP_MAP] {
            let at = src.floor_char_boundary(cut.min(src.len()));
            let mut m = src.to_string();
            m.insert_str(at, &junk);
            let _ = DistroList::parse(&m);
            let _ = IsoList::parse(&m);
            let _ = AppMap::parse(&m);
            let t = &src[..at];
            let _ = DistroList::parse(t);
            let _ = IsoList::parse(t);
            let _ = AppMap::parse(t);
        }
    }

    // ---- checksums / URLs ------------------------------------------------------------------
    #[test]
    fn checksums_yield_only_lowercase_sha256(s in texty()) {
        for (name, hash) in parse_checksums(&s) {
            prop_assert_eq!(hash.len(), 64);
            prop_assert!(hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            prop_assert!(!name.is_empty() && !name.contains('/') && !name.contains('\\'));
        }
    }

    #[test]
    fn checksum_gnu_lines_roundtrip(hash in "[0-9a-fA-F]{64}", name in "[A-Za-z0-9._-]{1,40}", star in any::<bool>()) {
        let line = format!("{hash} {}{name}\n", if star { "*" } else { " " });
        let got = parse_checksums(&line);
        prop_assert_eq!(got, vec![(name, hash.to_ascii_lowercase())]);
    }

    #[test]
    fn host_of_never_returns_userinfo_or_port(s in texty()) {
        if let Some(h) = iso_host_of(&s) {
            prop_assert!(s.starts_with("https://"));
            prop_assert!(!h.is_empty() && !h.contains(['@', '/', '?', '#', ':']));
        }
    }

    #[test]
    fn host_of_ignores_userinfo_tricks(host in "[a-z]{1,10}\\.[a-z]{2,3}", evil in "[a-z]{1,10}\\.[a-z]{2,3}") {
        let url = format!("https://{host}:pw@{evil}/x");
        prop_assert_eq!(iso_host_of(&url), Some(evil.as_str()));
    }

    // ---- winget / names --------------------------------------------------------------------
    #[test]
    fn winget_table_never_panics(s in texty()) {
        let _ = parse_winget_table(&s);
    }

    #[test]
    fn winget_table_survives_garbled_real_output(prefix in "[ -~\\r\\n]{0,60}", rows in prop::collection::vec(("[A-Za-z ]{1,30}", "[A-Za-z.]{1,20}", "[0-9.]{1,10}"), 0..8), crlf in any::<bool>(), cut in 0usize..600) {
        let nl = if crlf { "\r\n" } else { "\n" };
        let mut t = format!("{prefix}{nl}Name                      Id                 Version  Source{nl}{}{nl}", "-".repeat(60));
        for (n, i, v) in &rows {
            t.push_str(&format!("{n:<26}{i:<19}{v:<9}winget{nl}"));
        }
        let t = &t[..t.floor_char_boundary(cut.min(t.len()))];
        let _ = parse_winget_table(t);
    }

    #[test]
    fn normalize_name_is_idempotent_and_tidy(s in any::<String>()) {
        let n = normalize_name(&s);
        prop_assert_eq!(normalize_name(&n), n.clone());
        prop_assert!(!n.starts_with(' ') && !n.ends_with(' ') && !n.contains("  "));
        prop_assert!(n.chars().all(|c| !c.is_control()));
    }

    #[test]
    fn steam_key_roundtrip(id in any::<u32>()) {
        prop_assert_eq!(steam_appid_from_key(&format!("Steam App {id}")), Some(id));
    }

    #[test]
    fn steam_key_never_panics(s in any::<String>()) {
        let _ = steam_appid_from_key(&s);
    }

    #[test]
    fn pattern_match_never_panics(p in any::<String>(), n in any::<String>()) {
        if let Some(l) = pattern_match_len(&p, &n) {
            prop_assert!(l <= p.len());
        }
    }

    #[test]
    fn classify_never_panics_on_shipped_map(name in any::<String>()) {
        let map = AppMap::parse(EMBEDDED_APP_MAP).unwrap();
        let p = Program { name, publisher: None, version: None, sources: vec![], winget_id: None, steam_appid: None, microsoft_msix: false };
        let _ = classify(&p, &map);
    }

    // ---- user-profile formats --------------------------------------------------------------
    #[test]
    fn profiles_ini_never_panics_and_paths_are_nonempty(s in texty()) {
        for p in parse_profiles_ini(&s) {
            prop_assert!(!p.path.is_empty());
        }
    }

    #[test]
    fn profiles_ini_roundtrip(name in "[A-Za-z0-9 ]{1,12}", path in "Profiles/[a-z0-9.]{1,12}", rel in any::<bool>()) {
        let ini = format!("[General]\nStartWithLastProfile=1\n\n[Profile0]\nName={name}\nIsRelative={}\nPath={path}\n", rel as u8);
        let got = parse_profiles_ini(&ini);
        prop_assert_eq!(got.len(), 1);
        prop_assert_eq!(&got[0].path, &path);
        prop_assert_eq!(got[0].is_relative, rel);
    }

    #[test]
    fn vdf_readers_never_panic(s in texty()) {
        let _ = parse_library_folders(&s);
        let _ = parse_app_manifest(&s);
    }

    #[test]
    fn library_folders_roundtrip(paths in prop::collection::vec("[A-Za-z]:\\\\\\\\[A-Za-z0-9 ]{1,15}", 1..4)) {
        let mut vdf = String::from("\"libraryfolders\"\n{\n");
        for (i, p) in paths.iter().enumerate() {
            vdf.push_str(&format!("\t\"{i}\"\n\t{{\n\t\t\"path\"\t\t\"{p}\"\n\t}}\n"));
        }
        vdf.push_str("}\n");
        let got = parse_library_folders(&vdf);
        prop_assert_eq!(got.len(), paths.len());
    }

    #[test]
    fn app_manifest_roundtrip(id in any::<u32>(), name in "[A-Za-z0-9 ]{1,20}", size in any::<u64>()) {
        let acf = format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n\t\"SizeOnDisk\"\t\t\"{size}\"\n}}\n");
        let g = parse_app_manifest(&acf).unwrap();
        prop_assert_eq!((g.appid, g.name, g.size_on_disk), (id, name, size));
    }

    #[test]
    fn chromium_local_state_never_panics(s in texty()) {
        let _ = parse_chromium_local_state(&s);
    }

    // ---- quiz web page input ---------------------------------------------------------------
    #[test]
    fn quiz_result_never_panics(s in texty()) {
        let (list, _) = DistroList::load(None).unwrap();
        let _ = parse_result(&s, Some(&list));
        let _ = parse_result(&s, None);
    }

    #[test]
    fn quiz_result_oversize_is_refused(extra in 1usize..2000) {
        let big = " ".repeat(MAX_RESULT_BYTES + extra);
        prop_assert!(parse_result(&big, None).is_err());
    }

    #[test]
    fn quiz_distro_pick_must_exist(id in "[a-z0-9-]{1,30}") {
        let (list, _) = DistroList::load(None).unwrap();
        let payload = format!("{{\"version\":1,\"distro\":\"{id}\"}}");
        match parse_result(&payload, Some(&list)) {
            Ok(QuizResult::Distro(d)) => prop_assert!(list.distros.iter().any(|x| x.id == d)),
            Ok(_) => prop_assert!(false),
            Err(_) => {}
        }
    }

    #[test]
    fn base64_matches_reference_encoder(bytes in prop::collection::vec(any::<u8>(), 0..300), pad in any::<bool>()) {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut enc = String::new();
        for ch in bytes.chunks(3) {
            let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
            let k = ch.len() + 1;
            for i in 0..k { enc.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char); }
            if pad { for _ in k..4 { enc.push('='); } }
        }
        prop_assert_eq!(base64url_decode(&enc), Some(bytes));
    }

    #[test]
    fn base64_never_panics(s in any::<String>()) {
        let _ = base64url_decode(&s);
    }

    #[test]
    fn url_extraction_never_panics(s in texty()) {
        let _ = extract_from_url(&s);
        let _ = check_url(&s);
        let _ = host_of(&s);
    }

    #[test]
    fn external_open_only_https_or_mailto(s in any::<String>()) {
        if safe_to_open_externally(&s) {
            let l = s.to_ascii_lowercase();
            prop_assert!(l.starts_with("https://") || l.starts_with("mailto:"));
            prop_assert!(!s.chars().any(|c| c.is_whitespace() || c.is_control()));
        }
    }

    #[test]
    fn asset_paths_never_escape_root(req in texty()) {
        let root = Path::new("/quiz-root");
        if let Some(p) = resolve_asset(root, &req) {
            prop_assert!(p.starts_with(root));
            prop_assert!(!p.components().any(|c| matches!(c, std::path::Component::ParentDir)));
        }
    }

    #[test]
    fn asset_traversal_attempts_are_refused(a in "[a-z]{0,6}", b in "[a-z]{0,6}", enc in prop::sample::select(vec!["..", "%2e%2e", "%2E%2E", "..%2f", "%2e./", "..\\\\"])) {
        let root = Path::new("/quiz-root");
        let req = format!("/{a}/{enc}/{b}");
        prop_assert!(resolve_asset(root, &req).is_none());
    }

    #[test]
    fn local_quiz_never_navigates_to_dangerous_schemes(rest in "[ -~]{0,40}", scheme in prop::sample::select(vec!["file:", "javascript:", "data:", "ms-settings:", "FILE:", "JavaScript:"])) {
        let home = QuizSource::File("/q/index.html".into());
        let url = format!("{scheme}{rest}");
        if extract_from_url(&url).is_none() {
            prop_assert!(!matches!(nav_decision(&home, &url), Nav::Allow));
            let remote = QuizSource::Url("https://example.org/quiz".into());
            prop_assert!(!matches!(nav_decision(&remote, &url), Nav::Allow | Nav::OpenExternally));
        }
    }

    #[test]
    fn nav_decision_never_panics(t in texty()) {
        let _ = nav_decision(&QuizSource::File("/q/index.html".into()), &t);
        let _ = nav_decision(&QuizSource::Url("https://example.org/".into()), &t);
    }

    // ---- answers file ----------------------------------------------------------------------
    #[test]
    fn answers_file_never_panics(s in texty()) {
        if let Ok(a) = serde_json::from_str::<Answers>(&s) {
            let _ = validate_answers(&a);
        }
    }

    #[test]
    fn complete_valid_answers_always_validate(picks in prop::collection::vec(any::<prop::sample::Index>(), QUESTIONS.len())) {
        let a: Answers = QUESTIONS.iter().zip(&picks).map(|(q, i)| (q.id.to_string(), q.options[i.index(q.options.len())].key.to_string())).collect();
        prop_assert!(validate_answers(&a).is_ok());
        let mut extra = a.clone();
        extra.insert("no-such-question".into(), "x".into());
        prop_assert!(validate_answers(&extra).is_err());
    }

    // ---- firmware bytes --------------------------------------------------------------------
    #[test]
    fn gpt_entries_never_panic(buf in prop::collection::vec(any::<u8>(), 0..2048), count in 0u32..4096, size in 0u32..512) {
        for (slot, _) in parse_gpt_entries(&buf, count, size) {
            prop_assert!(slot >= 1 && slot <= count);
        }
    }

    #[test]
    fn load_option_readers_never_panic(opt in prop::collection::vec(any::<u8>(), 0..512), parts in prop::collection::vec((1u32..130, any::<[u8; 16]>()), 0..6)) {
        let _ = load_option_description(&opt);
        let parts: Vec<GptEntry> = parts;
        if let Some((fixed, _)) = fix_load_option(&opt, &parts) {
            prop_assert_eq!(fixed.len(), opt.len());
        }
    }

    // ---- bcdedit output / paths ------------------------------------------------------------
    #[test]
    fn bcd_guid_is_well_formed(s in texty()) {
        if let Some(g) = parse_bcd_guid(&s) {
            prop_assert_eq!(g.len(), 38);
            let ok = g.starts_with('{') && g.ends_with('}');
            prop_assert!(ok);
        }
    }

    #[test]
    fn bcd_guid_found_in_localised_output(pre in "[^{}]{0,40}", post in "[^{}]{0,40}", g in "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}") {
        let out = format!("{pre}{{{g}}}{post}");
        prop_assert_eq!(parse_bcd_guid(&out), Some(format!("{{{g}}}")));
    }

    #[test]
    fn path_within_is_reflexive_and_respects_boundaries(a in "[A-Za-z0-9 \\\\/:._-]{0,40}", b in "[A-Za-z0-9]{1,10}") {
        prop_assert!(path_within(Path::new(&a), Path::new(&a)));
        // "C:\\data2" is not inside "C:\\data".
        let outer = format!("C:\\{b}");
        let sibling = format!("C:\\{b}x");
        prop_assert!(!path_within(Path::new(&sibling), Path::new(&outer)));
        let inner = format!("{outer}\\sub");
        prop_assert!(path_within(Path::new(&inner), Path::new(&outer)));
    }

    // ---- migration progress log ------------------------------------------------------------
    #[test]
    fn progress_log_ignores_garbage(s in texty()) {
        let _ = parse_progress(&s);
    }
}

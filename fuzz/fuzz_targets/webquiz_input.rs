//! Everything a (possibly hostile) quiz web page or quiz.json can feed the app.
#![no_main]
use libfuzzer_sys::fuzz_target;
use linuxify::chooser::DistroList;
use linuxify::webquiz::*;
use std::path::Path;
use std::sync::OnceLock;

static DISTROS: OnceLock<Option<DistroList>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data).into_owned();
    let distros = DISTROS.get_or_init(|| DistroList::load(None).ok().map(|x| x.0)).as_ref();
    let _ = parse_result(&s, distros);
    let _ = base64url_decode(&s);
    let _ = extract_from_url(&s);
    let _ = check_url(&s);
    let _ = host_of(&s);
    let _ = serde_json::from_str::<QuizSettings>(&s);

    if safe_to_open_externally(&s) {
        let l = s.to_ascii_lowercase();
        assert!(l.starts_with("https://") || l.starts_with("mailto:"));
        assert!(!s.chars().any(|c| c.is_whitespace() || c.is_control()));
    }

    let root = Path::new("/quiz-root");
    if let Some(p) = resolve_asset(root, &s) {
        assert!(p.starts_with(root));
        assert!(!p.components().any(|c| matches!(c, std::path::Component::ParentDir)));
    }

    // A local-file quiz must never navigate to file:, javascript: or data: pages.
    let home = QuizSource::File(root.join("index.html"));
    let t = s.to_ascii_lowercase();
    if t.starts_with("file:") || t.starts_with("javascript:") || t.starts_with("data:") {
        if extract_from_url(&s).is_none() {
            assert!(!matches!(nav_decision(&home, &s), Nav::Allow));
        }
    }
    if let Ok(u) = check_url("https://example.org/quiz") {
        let _ = nav_decision(&QuizSource::Url(u), &s);
    }
});

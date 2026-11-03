//! The website quiz: pure logic for loading the owner's distro questionnaire
//! (a web page) inside the app and taking its result back.
//!
//! The web view itself (`webview.rs`, feature `webview`) is a thin shell around
//! this module. Everything that decides what is allowed lives here so it can be
//! unit-tested on any OS:
//!
//! * which address may be loaded (`QuizSource`),
//! * which navigations stay inside the window and which go to the browser
//!   (`nav_decision`),
//! * how the page hands its answers back (`parse_result`, `extract_from_url`).
//!
//! Everything the page sends is untrusted text. It is size-limited, parsed as
//! JSON and checked against the built-in questions or the distro list before
//! anything uses it. It can never carry a command, a path or a URL.

use crate::chooser::{self, Answers, DistroList, QUESTIONS};
use serde::{Deserialize, Serialize};

/// Largest result message accepted from the page.
pub const MAX_RESULT_BYTES: usize = 16 * 1024;
/// Query/fragment parameter a page can navigate to instead of using `window.ipc`.
pub const URL_PARAM: &str = "linuxify_result";
/// Environment variable that overrides the configured quiz address.
pub const ENV_URL: &str = "LINUXIFY_QUIZ_URL";
/// Settings file name (in the Linuxify folder, or next to the data files).
pub const SETTINGS_FILE: &str = "quiz.json";

/// Where the quiz page comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuizSource {
    /// A web address (https, or http for localhost during development).
    Url(String),
    /// A local HTML file the user (or a release) ships with the app.
    File(std::path::PathBuf),
}

/// `quiz.json`: `{ "url": "https://example.org/quiz/" }` or `{ "file": "quiz/index.html" }`.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QuizSettings {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
}

/// An address is only loaded when it is `https://`, or `http://` to this PC.
pub fn check_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.len() > 2048 || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("the quiz address is not a valid web address".into());
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return host_of(url).map(|_| url.to_string()).ok_or_else(|| "the quiz address has no host name".to_string());
    }
    if lower.starts_with("http://") {
        return match host_of(url).as_deref() {
            Some("localhost") | Some("127.0.0.1") => Ok(url.to_string()),
            _ => Err("the quiz address must start with https:// (http:// is only allowed for localhost)".into()),
        };
    }
    Err("the quiz address must start with https://".into())
}

/// Lower-case host name of an http(s) URL, without port or credentials.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None; // user:pass@host tricks are never needed here
    }
    let host = if let Some(v6) = authority.strip_prefix('[') { v6.split(']').next()? } else { authority.split(':').next()? };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Resolves the quiz source from (in order) a command-line value, `LINUXIFY_QUIZ_URL`,
/// and a `quiz.json` found in `dirs`. `Ok(None)` means no website quiz is set up.
pub fn resolve_source(cli_url: Option<&str>, env_url: Option<&str>, dirs: &[std::path::PathBuf]) -> Result<Option<QuizSource>, String> {
    if let Some(u) = cli_url.or(env_url).filter(|u| !u.trim().is_empty()) {
        return check_url(u).map(|u| Some(QuizSource::Url(u)));
    }
    for d in dirs {
        let p = d.join(SETTINGS_FILE);
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let s: QuizSettings = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("{} is not valid: {e}", p.display()))?;
        match (s.url, s.file) {
            (Some(u), None) => return check_url(&u).map(|u| Some(QuizSource::Url(u))),
            (None, Some(f)) => {
                let f = std::path::Path::new(&f);
                if f.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                    return Err(format!("{}: \"file\" must not contain '..'", p.display()));
                }
                let full = if f.is_absolute() { f.to_path_buf() } else { d.join(f) };
                return Ok(Some(QuizSource::File(full)));
            }
            (None, None) => continue,
            (Some(_), Some(_)) => return Err(format!("{}: set either \"url\" or \"file\", not both", p.display())),
        }
    }
    Ok(None)
}

/// What the window does with a navigation the page asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Nav {
    /// Load it in the quiz window.
    Allow,
    /// Do not load it here; hand it to the user's normal browser.
    OpenExternally,
    /// Do not load it at all.
    Block,
    /// It carries the quiz result; take it and close the window.
    Result(String),
}

/// Decides one navigation. `home` is the quiz address (the origin that may stay in the window).
pub fn nav_decision(home: &QuizSource, target: &str) -> Nav {
    if let Some(r) = extract_from_url(target) {
        return Nav::Result(r);
    }
    let t = target.to_ascii_lowercase();
    if t == "about:blank" {
        return Nav::Allow;
    }
    match home {
        QuizSource::File(_) => {
            // A local quiz is served by the app itself; its own pages stay, https links go to the browser.
            if is_asset_url(&t) {
                Nav::Allow
            } else if t.starts_with("https://") {
                Nav::OpenExternally
            } else {
                Nav::Block
            }
        }
        QuizSource::Url(u) => {
            if t.starts_with("https://") || t.starts_with("http://") {
                let same = host_of(&t).is_some() && host_of(&t) == host_of(u);
                if same && (t.starts_with("https://") || host_of(u).is_some_and(|h| h == "localhost" || h == "127.0.0.1")) {
                    Nav::Allow
                } else if t.starts_with("https://") {
                    Nav::OpenExternally
                } else {
                    Nav::Block
                }
            } else if t.starts_with("mailto:") {
                Nav::OpenExternally
            } else {
                // file:, javascript:, data:, ms-settings:, custom app schemes ... never.
                Nav::Block
            }
        }
    }
}

/// Host a local quiz is served from. WebView2 maps the custom scheme `linuxify://` to
/// `http://linuxify.localhost/`; WebKitGTK keeps `linuxify://localhost/`.
pub const ASSET_HOST: &str = "linuxify.localhost";
/// Custom protocol name used to serve a local quiz folder (never `file://`, which wry's
/// message bridge cannot use).
pub const ASSET_SCHEME: &str = "linuxify";

/// True for the addresses a locally served quiz uses.
fn is_asset_url(url_lower: &str) -> bool {
    url_lower.starts_with("linuxify://") || (url_lower.starts_with("http://") && host_of(url_lower).as_deref() == Some(ASSET_HOST))
}

/// Maps the path of a request for the local quiz to a file inside `root`. Refuses
/// anything that could leave the folder: `..`, backslashes, drive letters, encoded dots.
pub fn resolve_asset(root: &std::path::Path, request_path: &str) -> Option<std::path::PathBuf> {
    let p = request_path.split(['?', '#']).next().unwrap_or("");
    let p = p.trim_start_matches('/');
    let p = if p.is_empty() { "index.html" } else { p };
    let lower = p.to_ascii_lowercase();
    if p.contains(['\\', ':', '\0']) || lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") {
        return None;
    }
    let mut out = root.to_path_buf();
    for part in p.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

/// Content type for the few file types a quiz page needs; anything else is refused.
pub fn asset_mime(path: &std::path::Path) -> Option<&'static str> {
    Some(match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => return None,
    })
}

/// Only these schemes are ever opened in the user's browser.
pub fn safe_to_open_externally(url: &str) -> bool {
    let l = url.to_ascii_lowercase();
    (l.starts_with("https://") || l.starts_with("mailto:")) && !l.chars().any(|c| c.is_control() || c.is_whitespace()) && l.len() <= 2048
}

/// Looks for `linuxify_result=<base64url JSON>` in the query or fragment of `url`.
pub fn extract_from_url(url: &str) -> Option<String> {
    let after = url.split_once('#').map(|x| x.1).into_iter().chain(url.split_once('?').map(|x| x.1.split('#').next().unwrap_or("")));
    for part in after {
        for kv in part.split('&') {
            if let Some(v) = kv.strip_prefix(URL_PARAM).and_then(|r| r.strip_prefix('=')) {
                let bytes = base64url_decode(v)?;
                return String::from_utf8(bytes).ok();
            }
        }
    }
    None
}

/// base64 / base64url decoder (padding optional). No crate needed for this.
pub fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
        if out.len() > MAX_RESULT_BYTES * 2 {
            return None;
        }
    }
    Some(out)
}

/// What a quiz page can hand back.
#[derive(Debug, PartialEq, Eq)]
pub enum QuizResult {
    /// Answers to Linuxify's own questions; the ranking is computed here with the scan.
    Answers(Answers),
    /// The page already chose a distro (its id must exist in distros.json).
    Distro(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    answers: Option<Answers>,
    #[serde(default)]
    distro: Option<String>,
}

/// Parses and validates what the page sent. `distros` is used to check a direct pick.
pub fn parse_result(payload: &str, distros: Option<&DistroList>) -> Result<QuizResult, String> {
    if payload.len() > MAX_RESULT_BYTES {
        return Err("the quiz result is too large".into());
    }
    let w: Wire = serde_json::from_str(payload.trim_start_matches('\u{feff}')).map_err(|e| format!("the quiz result is not valid: {e}"))?;
    if w.version.is_some_and(|v| v != 1) {
        return Err("the quiz result is from a newer version this app does not understand".into());
    }
    match (w.answers, w.distro) {
        (Some(a), None) => {
            chooser::validate_answers(&a).map_err(|e| format!("the quiz result has bad answers: {e}"))?;
            Ok(QuizResult::Answers(a))
        }
        (None, Some(id)) => {
            if id.len() > 64 || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                return Err("the quiz result names an invalid distro id".into());
            }
            if let Some(list) = distros {
                if !list.distros.iter().any(|d| d.id == id) {
                    return Err(format!("the quiz picked '{id}', which this version of Linuxify does not know"));
                }
            }
            Ok(QuizResult::Distro(id))
        }
        (Some(_), Some(_)) => Err("the quiz result has both answers and a distro; send one".into()),
        (None, None) => Err("the quiz result has neither answers nor a distro".into()),
    }
}

/// The questions and option keys a web quiz can send, as JSON (`linuxify quiz-spec`).
pub fn spec_json() -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "how_to_send": {
            "ipc": "window.ipc.postMessage(JSON.stringify({version:1, answers:{...}}))",
            "url": format!("navigate to any URL with #{URL_PARAM}=<base64url of the same JSON>"),
        },
        "or_pick_a_distro": {"version": 1, "distro": "linux-mint"},
        "questions": QUESTIONS.iter().map(|q| serde_json::json!({
            "id": q.id,
            "text": q.text,
            "options": q.options.iter().map(|o| serde_json::json!({"key": o.key, "label": o.label})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// Maps a quiz result to the answers used for ranking. A direct distro pick has no
/// answers, so it returns None and the caller shows that distro first.
pub fn answers_of(r: &QuizResult) -> Option<&Answers> {
    match r {
        QuizResult::Answers(a) => Some(a),
        QuizResult::Distro(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_answers() -> Answers {
        QUESTIONS.iter().map(|q| (q.id.to_string(), q.options[0].key.to_string())).collect()
    }
    fn distros() -> DistroList {
        DistroList::parse(chooser::EMBEDDED_DISTROS).unwrap()
    }
    fn b64(s: &str) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let b = s.as_bytes();
        let mut o = String::new();
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..=c.len() {
                o.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
        }
        o
    }

    #[test]
    fn urls_must_be_https_or_local() {
        assert!(check_url("https://example.org/quiz/").is_ok());
        assert!(check_url("http://localhost:8080/").is_ok());
        assert!(check_url("http://127.0.0.1/").is_ok());
        for bad in ["http://example.org/", "file:///C:/x.html", "javascript:alert(1)", "https://", "https://user:pw@example.org/", "https://exa mple.org", "ftp://x.org", "", "https://a.org/\nx"] {
            assert!(check_url(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn host_parsing() {
        assert_eq!(host_of("https://Example.ORG:8443/a?b#c").as_deref(), Some("example.org"));
        assert_eq!(host_of("https://[::1]:80/").as_deref(), Some("::1"));
        assert_eq!(host_of("https://evil.org@good.org/"), None);
    }

    #[test]
    fn navigation_policy() {
        let home = QuizSource::Url("https://quiz.example.org/start".into());
        assert_eq!(nav_decision(&home, "https://quiz.example.org/q/2"), Nav::Allow);
        assert_eq!(nav_decision(&home, "https://other.org/"), Nav::OpenExternally);
        assert_eq!(nav_decision(&home, "https://quiz.example.org.evil.org/"), Nav::OpenExternally);
        assert_eq!(nav_decision(&home, "http://quiz.example.org/"), Nav::Block);
        assert_eq!(nav_decision(&home, "file:///C:/Windows/win.ini"), Nav::Block);
        assert_eq!(nav_decision(&home, "javascript:alert(1)"), Nav::Block);
        assert_eq!(nav_decision(&home, "ms-settings:network"), Nav::Block);
        assert_eq!(nav_decision(&home, "mailto:a@b.org"), Nav::OpenExternally);
        let local = QuizSource::File("C:/x/quiz/index.html".into());
        assert_eq!(nav_decision(&local, "linuxify://localhost/index.html"), Nav::Allow);
        assert_eq!(nav_decision(&local, "http://linuxify.localhost/q2.html"), Nav::Allow);
        assert_eq!(nav_decision(&local, "http://linuxify.localhost.evil.org/"), Nav::Block);
        assert_eq!(nav_decision(&local, "file:///C:/Windows/win.ini"), Nav::Block);
        assert_eq!(nav_decision(&local, "https://example.org/"), Nav::OpenExternally);
        assert_eq!(nav_decision(&local, "http://example.org/"), Nav::Block);
    }

    #[test]
    fn result_in_url_fragment_and_query() {
        let json = serde_json::json!({"version":1,"distro":"linux-mint"}).to_string();
        let enc = b64(&json);
        let home = QuizSource::Url("https://q.example.org/".into());
        assert_eq!(nav_decision(&home, &format!("https://q.example.org/done#{URL_PARAM}={enc}")), Nav::Result(json.clone()));
        assert_eq!(nav_decision(&home, &format!("https://anything.org/?x=1&{URL_PARAM}={enc}")), Nav::Result(json));
        assert_eq!(extract_from_url("https://a.org/#linuxify_result=!!!"), None);
        assert_eq!(extract_from_url("https://a.org/"), None);
    }

    #[test]
    fn base64_roundtrip_and_limits() {
        for s in ["", "a", "ab", "abc", "abcd", "héllo wörld ✓"] {
            assert_eq!(base64url_decode(&b64(s)).unwrap(), s.as_bytes());
        }
        assert_eq!(base64url_decode("YWJj=="), Some(b"abc".to_vec()));
        assert!(base64url_decode("a b").is_none());
        assert!(base64url_decode(&"A".repeat(MAX_RESULT_BYTES * 4)).is_none());
    }

    #[test]
    fn answers_result_is_validated() {
        let a = all_answers();
        let ok = serde_json::json!({"version":1,"answers":a}).to_string();
        assert_eq!(parse_result(&ok, None).unwrap(), QuizResult::Answers(a.clone()));
        // Unknown option, missing question, extra question, wrong version, junk.
        let mut bad = a.clone();
        bad.insert("gaming".into(), "mega".into());
        assert!(parse_result(&serde_json::json!({"answers":bad}).to_string(), None).is_err());
        let mut missing = a.clone();
        missing.remove("gaming");
        assert!(parse_result(&serde_json::json!({"answers":missing}).to_string(), None).is_err());
        let mut extra = a.clone();
        extra.insert("favourite_colour".into(), "red".into());
        assert!(parse_result(&serde_json::json!({"answers":extra}).to_string(), None).is_err());
        assert!(parse_result(&serde_json::json!({"version":2,"answers":a}).to_string(), None).is_err());
        assert!(parse_result("not json", None).is_err());
        assert!(parse_result("{}", None).is_err());
        assert!(parse_result(&serde_json::json!({"answers":a,"distro":"linux-mint"}).to_string(), None).is_err());
        assert!(parse_result(&serde_json::json!({"answers":a,"cmd":"format c:"}).to_string(), None).is_err(), "unknown fields are refused");
    }

    #[test]
    fn distro_result_is_checked_against_the_list() {
        let l = distros();
        assert_eq!(parse_result(r#"{"distro":"linux-mint"}"#, Some(&l)).unwrap(), QuizResult::Distro("linux-mint".into()));
        assert!(parse_result(r#"{"distro":"windows-12"}"#, Some(&l)).is_err());
        assert!(parse_result(r#"{"distro":"../../etc"}"#, Some(&l)).is_err());
        assert!(parse_result(r#"{"distro":"Linux Mint"}"#, None).is_err());
    }

    #[test]
    fn oversized_results_are_refused() {
        let big = format!(r#"{{"distro":"{}"}}"#, "a".repeat(MAX_RESULT_BYTES));
        assert!(parse_result(&big, None).is_err());
    }

    #[test]
    fn settings_resolution() {
        let dir = std::env::temp_dir().join(format!("lnx-quiz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dirs = vec![dir.clone()];
        assert_eq!(resolve_source(None, None, &dirs).unwrap(), None);
        std::fs::write(dir.join(SETTINGS_FILE), r#"{"url":"https://q.example.org/"}"#).unwrap();
        assert_eq!(resolve_source(None, None, &dirs).unwrap(), Some(QuizSource::Url("https://q.example.org/".into())));
        // Command line beats environment beats file.
        assert_eq!(resolve_source(Some("https://cli.org/"), Some("https://env.org/"), &dirs).unwrap(), Some(QuizSource::Url("https://cli.org/".into())));
        assert_eq!(resolve_source(None, Some("https://env.org/"), &dirs).unwrap(), Some(QuizSource::Url("https://env.org/".into())));
        assert!(resolve_source(Some("http://evil.org/"), None, &dirs).is_err());
        std::fs::write(dir.join(SETTINGS_FILE), r#"{"file":"quiz/index.html"}"#).unwrap();
        assert_eq!(resolve_source(None, None, &dirs).unwrap(), Some(QuizSource::File(dir.join("quiz/index.html"))));
        std::fs::write(dir.join(SETTINGS_FILE), r#"{"file":"../secret.html"}"#).unwrap();
        assert!(resolve_source(None, None, &dirs).is_err());
        std::fs::write(dir.join(SETTINGS_FILE), r#"{"url":"https://a.org/","file":"x.html"}"#).unwrap();
        assert!(resolve_source(None, None, &dirs).is_err());
        std::fs::write(dir.join(SETTINGS_FILE), "\u{feff}{}").unwrap();
        assert_eq!(resolve_source(None, None, &dirs).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn asset_paths_cannot_escape_the_quiz_folder() {
        let root = std::path::Path::new("/q");
        assert_eq!(resolve_asset(root, "/"), Some("/q/index.html".into()));
        assert_eq!(resolve_asset(root, "//etc/passwd"), Some("/q/etc/passwd".into()), "stays inside the folder");
        assert_eq!(resolve_asset(root, "/css/a.css?v=2"), Some("/q/css/a.css".into()));
        for bad in ["/../x", "/a/../../x", "/a/%2e%2e/x", "/..%5cx", "/a\\b", "/C:/Windows/win.ini", "/a//b", "/a\0b", "/%2Fetc"] {
            assert_eq!(resolve_asset(root, bad), None, "{bad:?}");
        }
        assert_eq!(asset_mime(std::path::Path::new("a.HTML")), Some("text/html; charset=utf-8"));
        assert_eq!(asset_mime(std::path::Path::new("run.exe")), None);
        assert_eq!(asset_mime(std::path::Path::new("noext")), None);
    }

    #[test]
    fn external_open_is_limited_to_safe_schemes() {
        assert!(safe_to_open_externally("https://example.org/a"));
        assert!(safe_to_open_externally("mailto:a@b.org"));
        assert!(!safe_to_open_externally("file:///C:/Windows/System32/calc.exe"));
        assert!(!safe_to_open_externally("https://a.org/ & calc"));
        assert!(!safe_to_open_externally("ms-msdt:/id"));
    }

    #[test]
    fn spec_lists_every_question() {
        let s = spec_json();
        assert_eq!(s["questions"].as_array().unwrap().len(), QUESTIONS.len());
    }
}

//! Static validation of the shipped data files (no network, no disk access).
//!
//! `AppMap::lint` already covers the basics; this file adds the checks that catch the slow rot
//! in a large hand-edited catalog: unknown keys, duplicates, malformed or placeholder package
//! names, and download links that cannot work. Checks work on raw JSON so a typo in a key is
//! caught rather than silently ignored by serde.

use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

const APP_MAP: &str = include_str!("../data/app_map.json");
const ISOS: &str = include_str!("../data/isos.json");
const DISTROS: &str = include_str!("../data/distros.json");

const ENTRY_KEYS: [&str; 9] = ["id", "name", "match", "category", "status", "linux", "alternatives", "notes", "link"];
const LINUX_KEYS: [&str; 5] = ["flatpak", "apt", "dnf", "snap", "url"];
const ALT_KEYS: [&str; 3] = ["name", "flatpak", "note"];
const STATUSES: [&str; 5] = ["native", "alternative", "wine_ok", "blocked", "unknown"];

fn json(s: &str) -> Value {
    serde_json::from_str(s).expect("data file is valid JSON")
}

fn apps() -> Vec<Value> {
    json(APP_MAP)["apps"].as_array().expect("apps is an array").clone()
}

fn id_of(e: &Value) -> String {
    e["id"].as_str().unwrap_or("<no id>").to_string()
}

fn unknown_keys(obj: &Value, allowed: &[&str], who: &str, problems: &mut Vec<String>) {
    if let Some(m) = obj.as_object() {
        for k in m.keys() {
            if !allowed.contains(&k.as_str()) {
                problems.push(format!("{who}: unknown key '{k}'"));
            }
        }
    }
}

fn assert_clean(problems: Vec<String>, what: &str) {
    assert!(problems.is_empty(), "{what}: {} problem(s):\n  - {}", problems.len(), problems.join("\n  - "));
}

/// Debian/Fedora package names: lowercase letters, digits and `+ - .`, starting alphanumeric.
fn is_distro_package(s: &str) -> bool {
    s.len() >= 2 && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+-._".contains(c))
}

/// Snap names: lowercase letters, digits and hyphens, no leading/trailing/double hyphen.
fn is_snap_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && !s.starts_with('-') && !s.ends_with('-') && !s.contains("--") && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_placeholder(s: &str) -> bool {
    let l = s.to_lowercase();
    let words: Vec<&str> = l.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    ["todo", "tbd", "fixme", "xxx", "placeholder", "changeme", "lorem"].iter().any(|p| words.contains(p)) || l.contains("example.com") || l.contains("example.org")
}

/// A https URL with a real host: no spaces, no localhost / IP literal / URL shortener / credentials.
fn url_problem(u: &str) -> Option<String> {
    let rest = u.strip_prefix("https://").map_or_else(|| Some("not https://".to_string()), |_| None);
    if rest.is_some() {
        return rest;
    }
    let after = &u["https://".len()..];
    if after.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Some("contains whitespace".into());
    }
    let host = after.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains('@') {
        return Some("embeds credentials".into());
    }
    let host_only = host.split(':').next().unwrap_or("");
    if host_only.is_empty() || !host_only.contains('.') || host_only.starts_with('.') || host_only.ends_with('.') {
        return Some(format!("host '{host_only}' is not a domain name"));
    }
    if host_only.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Some("IP address instead of a domain".into());
    }
    if host_only.ends_with(".local") || host_only.ends_with(".internal") || host_only.ends_with(".test") || host_only == "localhost" {
        return Some("non-public host".into());
    }
    if ["bit.ly", "tinyurl.com", "t.co", "goo.gl", "ow.ly", "is.gd", "cutt.ly"].contains(&host_only) {
        return Some("URL shortener".into());
    }
    if is_placeholder(u) {
        return Some("placeholder text".into());
    }
    None
}

#[test]
fn catalog_is_not_trivially_small() {
    // Guards against an accidental truncation or an emptied file.
    assert!(apps().len() >= 590, "catalog shrank to {} entries (was 590+)", apps().len());
}

#[test]
fn every_entry_has_the_right_shape() {
    let mut p = Vec::new();
    for e in apps() {
        let id = id_of(&e);
        unknown_keys(&e, &ENTRY_KEYS, &id, &mut p);
        for k in ["id", "name", "category", "status"] {
            if !e[k].as_str().is_some_and(|s| !s.trim().is_empty()) {
                p.push(format!("{id}: '{k}' must be a non-empty string"));
            }
        }
        if let Some(n) = e["name"].as_str() {
            if n != n.trim() {
                p.push(format!("{id}: name has leading/trailing whitespace"));
            }
        }
        if !e["match"].as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|x| x.as_str().is_some())) {
            p.push(format!("{id}: 'match' must be a non-empty array of strings"));
        }
        if !e["alternatives"].is_array() {
            p.push(format!("{id}: 'alternatives' must be an array (use [])"));
        }
        let status = e["status"].as_str().unwrap_or("");
        if !STATUSES.contains(&status) {
            p.push(format!("{id}: unknown status '{status}'"));
        }
        for k in ["notes", "link"] {
            if !e[k].is_null() && !e[k].is_string() {
                p.push(format!("{id}: '{k}' must be a string"));
            }
        }
        if let Some(n) = e["notes"].as_str() {
            if n.trim().len() < 15 {
                p.push(format!("{id}: note is too short to help anyone"));
            }
            if is_placeholder(n) {
                p.push(format!("{id}: note contains placeholder text"));
            }
        }
        if !e["linux"].is_null() {
            unknown_keys(&e["linux"], &LINUX_KEYS, &format!("{id}.linux"), &mut p);
            for k in LINUX_KEYS {
                if !e["linux"][k].is_null() && !e["linux"][k].is_string() {
                    p.push(format!("{id}.linux.{k} must be a string"));
                }
            }
        }
        for (i, a) in e["alternatives"].as_array().into_iter().flatten().enumerate() {
            let who = format!("{id}.alternatives[{i}]");
            unknown_keys(a, &ALT_KEYS, &who, &mut p);
            if !a["name"].as_str().is_some_and(|s| !s.trim().is_empty()) {
                p.push(format!("{who}: needs a name"));
            }
        }
    }
    assert_clean(p, "schema");
}

#[test]
fn no_duplicates() {
    let mut p = Vec::new();
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut patterns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in apps() {
        let id = id_of(&e);
        *ids.entry(id.clone()).or_default() += 1;
        let name = e["name"].as_str().unwrap_or("").to_lowercase();
        if let Some(other) = names.insert(name.clone(), id.clone()) {
            p.push(format!("name '{name}' used by both '{other}' and '{id}'"));
        }
        let mut seen = HashSet::new();
        for pat in e["match"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
            if !seen.insert(pat) {
                p.push(format!("{id}: pattern '{pat}' listed twice"));
            }
            patterns.entry(pat.to_string()).or_default().push(id.clone());
        }
        let mut alt_names = HashSet::new();
        for a in e["alternatives"].as_array().into_iter().flatten() {
            let n = a["name"].as_str().unwrap_or("").to_lowercase();
            if !alt_names.insert(n.clone()) {
                p.push(format!("{id}: alternative '{n}' listed twice"));
            }
        }
    }
    p.extend(ids.into_iter().filter(|(_, n)| *n > 1).map(|(id, n)| format!("id '{id}' appears {n} times")));
    // The same pattern in two entries means one of them can never be the best match.
    p.extend(patterns.into_iter().filter(|(_, v)| v.len() > 1).map(|(pat, v)| format!("pattern '{pat}' is claimed by {}", v.join(", "))));
    assert_clean(p, "duplicates");
}

#[test]
fn package_names_are_well_formed() {
    let mut p = Vec::new();
    for e in apps() {
        let id = id_of(&e);
        let l = &e["linux"];
        if let Some(s) = l["apt"].as_str() {
            if !is_distro_package(s) {
                p.push(format!("{id}: apt name '{s}' is malformed"));
            }
        }
        if let Some(s) = l["dnf"].as_str() {
            // RPM names may be mixed case (VirtualBox) or a single letter (R).
            if !is_distro_package(&s.to_lowercase()) && s.len() > 1 {
                p.push(format!("{id}: dnf name '{s}' is malformed"));
            }
        }
        if let Some(s) = l["snap"].as_str() {
            if !is_snap_name(s) {
                p.push(format!("{id}: snap name '{s}' is malformed"));
            }
        }
        let flatpaks = [l["flatpak"].as_str()].into_iter().chain(e["alternatives"].as_array().into_iter().flatten().map(|a| a["flatpak"].as_str())).flatten();
        for f in flatpaks {
            if !linuxify::mapping::is_flatpak_id(f) {
                p.push(format!("{id}: flatpak id '{f}' is malformed"));
            }
        }
        for k in LINUX_KEYS {
            if let Some(s) = l[k].as_str() {
                if is_placeholder(s) {
                    p.push(format!("{id}: linux.{k} '{s}' looks like a placeholder"));
                }
            }
        }
        // A reverse-DNS id in an apt/dnf/snap slot is almost always a pasted Flatpak id.
        for k in ["apt", "dnf", "snap"] {
            if let Some(s) = l[k].as_str() {
                if s.matches('.').count() >= 2 && linuxify::mapping::is_flatpak_id(s) {
                    p.push(format!("{id}: linux.{k} '{s}' looks like a Flatpak id"));
                }
            }
        }
    }
    assert_clean(p, "package names");
}

#[test]
fn flatpak_ids_are_not_reused_by_unrelated_entries() {
    // Several entries may legitimately share one Flatpak (editions of one program), but never
    // more than a handful; a large cluster means a copy/paste slip.
    let mut by_flatpak: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in apps() {
        if let Some(f) = e["linux"]["flatpak"].as_str() {
            by_flatpak.entry(f.to_string()).or_default().push(id_of(&e));
        }
    }
    let p: Vec<String> = by_flatpak.into_iter().filter(|(_, v)| v.len() > 3).map(|(f, v)| format!("flatpak '{f}' used by {} entries: {}", v.len(), v.join(", "))).collect();
    assert_clean(p, "flatpak reuse");
}

#[test]
fn links_and_download_urls_are_plausible() {
    let mut p = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for e in apps() {
        let id = id_of(&e);
        let urls = [("link", e["link"].as_str()), ("linux.url", e["linux"]["url"].as_str())];
        for (k, u) in urls {
            if let Some(u) = u {
                if let Some(why) = url_problem(u) {
                    p.push(format!("{id}: {k} '{u}': {why}"));
                }
                if k == "linux.url" {
                    if let Some(other) = seen.insert(u.to_string(), id.clone()) {
                        if other != id {
                            p.push(format!("{id}: linux.url '{u}' also used by '{other}'"));
                        }
                    }
                }
            }
        }
    }
    assert_clean(p, "urls");
}

#[test]
fn shipped_data_passes_the_runtime_linter() {
    let map = linuxify::mapping::AppMap::parse(APP_MAP).expect("app_map.json parses and validates");
    assert_clean(map.lint(), "AppMap::lint");
    assert_eq!(map.apps.len(), apps().len());
}

#[test]
fn iso_sources_only_point_at_their_allowed_hosts() {
    let isos = json(ISOS);
    let distro_ids: HashSet<String> = json(DISTROS)["distros"].as_array().unwrap().iter().map(id_of).collect();
    let mut p = Vec::new();
    let mut seen = HashSet::new();
    for s in isos["sources"].as_array().expect("sources array") {
        let d = s["distro"].as_str().unwrap_or("<none>");
        if !seen.insert(d.to_string()) {
            p.push(format!("iso source for '{d}' listed twice"));
        }
        if !distro_ids.contains(d) {
            p.push(format!("iso source '{d}' is not in distros.json"));
        }
        let url = s["checksum_url"].as_str().unwrap_or("");
        if let Some(why) = url_problem(url) {
            p.push(format!("{d}: checksum_url '{url}': {why}"));
        }
        let host = url.trim_start_matches("https://").split('/').next().unwrap_or("");
        let allowed: Vec<&str> = s["allowed_hosts"].as_array().into_iter().flatten().filter_map(|h| h.as_str()).collect();
        if allowed.is_empty() {
            p.push(format!("{d}: allowed_hosts is empty"));
        }
        if !allowed.contains(&host) {
            p.push(format!("{d}: checksum host '{host}' is not in allowed_hosts {allowed:?}"));
        }
        for h in &allowed {
            if h.contains('/') || h.contains(':') || h.contains('*') || h.is_empty() {
                p.push(format!("{d}: allowed host '{h}' must be a bare hostname"));
            }
        }
        if s["suffix"].as_str().is_none_or(|x| !x.ends_with(".iso")) {
            p.push(format!("{d}: suffix must end in .iso"));
        }
    }
    assert_clean(p, "isos.json");
}

#[test]
fn distro_entries_are_consistent() {
    let d = json(DISTROS);
    let mut p = Vec::new();
    let mut seen = HashSet::new();
    for e in d["distros"].as_array().expect("distros array") {
        let id = id_of(e);
        if !seen.insert(id.clone()) {
            p.push(format!("duplicate distro id '{id}'"));
        }
        if let Some(u) = e["homepage"].as_str() {
            if let Some(why) = url_problem(u) {
                p.push(format!("{id}: homepage '{u}': {why}"));
            }
        } else {
            p.push(format!("{id}: missing homepage"));
        }
        for (k, v) in e["traits"].as_object().into_iter().flatten() {
            if !v.as_i64().is_some_and(|n| (0..=5).contains(&n)) {
                p.push(format!("{id}: trait '{k}' must be an integer 0-5"));
            }
        }
    }
    assert_clean(p, "distros.json");
}

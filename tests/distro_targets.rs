//! Static validation of the distro-target layer (`distros/`).
//!
//! Nothing here touches the network or the system. Every mapping in
//! `distros/packages/*.json` is checked against the schema in
//! `docs/distro-targets.md` and cross-checked against `data/app_map.json`.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const TARGETS: [&str; 5] = ["fedora", "arch", "debian", "ubuntu", "opensuse"];
const CONFIDENCE: [&str; 3] = ["high", "medium", "low"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read_json(p: &Path) -> Value {
    let s = fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("parse {}: {e}", p.display()))
}

fn shard_files(dir: &str) -> Vec<PathBuf> {
    let d = root().join("distros").join(dir);
    let mut v: Vec<PathBuf> = match fs::read_dir(&d) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map_or(false, |x| x == "json"))
            .collect(),
        Err(_) => vec![],
    };
    v.sort();
    v
}

/// id -> (target -> entry), merged across shards; duplicates are reported.
fn load_shards(dir: &str) -> (BTreeMap<String, Value>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut errs = vec![];
    for f in shard_files(dir) {
        let v = read_json(&f);
        if v["schema_version"] != 1 {
            errs.push(format!("{}: schema_version must be 1", f.display()));
        }
        let Some(apps) = v["apps"].as_object() else {
            errs.push(format!("{}: missing \"apps\" object", f.display()));
            continue;
        };
        for (id, e) in apps {
            if out.insert(id.clone(), e.clone()).is_some() {
                errs.push(format!(
                    "{}: duplicate app id '{id}' across shards",
                    f.display()
                ));
            }
        }
    }
    (out, errs)
}

struct Catalog {
    /// id -> (category, linux object or Null)
    apps: BTreeMap<String, (String, Value)>,
    /// id -> alternatives (name, flatpak)
    alts: BTreeMap<String, Vec<(String, Option<String>)>>,
}

fn catalog() -> Catalog {
    let v = read_json(&root().join("data/app_map.json"));
    let mut apps = BTreeMap::new();
    let mut alts = BTreeMap::new();
    for a in v["apps"].as_array().unwrap() {
        let id = a["id"].as_str().unwrap().to_string();
        apps.insert(
            id.clone(),
            (
                a["category"].as_str().unwrap().to_string(),
                a["linux"].clone(),
            ),
        );
        let list: Vec<(String, Option<String>)> = a["alternatives"]
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|x| {
                        (
                            x["name"].as_str().unwrap().to_string(),
                            x["flatpak"].as_str().map(String::from),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !list.is_empty() {
            alts.insert(id, list);
        }
    }
    Catalog { apps, alts }
}

fn is_pkg_name(s: &str) -> bool {
    let mut c = s.chars();
    match c.next() {
        Some(f) if f.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    s.len() <= 100
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "+._@-".contains(c))
}

fn is_flatpak_id(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() >= 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
        && !parts[0].chars().next().unwrap().is_ascii_digit()
}

fn allowed_sources(target: &str) -> &'static [&'static str] {
    match target {
        "fedora" => &[
            "repo",
            "rpmfusion",
            "copr",
            "vendor-repo",
            "flatpak",
            "download",
            "none",
        ],
        "arch" => &["repo", "aur", "flatpak", "download", "none"],
        "debian" => &["repo", "vendor-repo", "flatpak", "download", "none"],
        "ubuntu" => &[
            "repo",
            "ppa",
            "snap",
            "vendor-repo",
            "flatpak",
            "download",
            "none",
        ],
        "opensuse" => &[
            "repo",
            "packman",
            "obs",
            "vendor-repo",
            "flatpak",
            "download",
            "none",
        ],
        _ => &[],
    }
}

/// Validate one target entry; returns problems.
fn check_entry(label: &str, target: &str, e: &Value, targets: &Value) -> Vec<String> {
    let mut p = vec![];
    let Some(o) = e.as_object() else {
        return vec![format!("{label}: not an object")];
    };
    for k in o.keys() {
        if ![
            "source",
            "pkg",
            "confidence",
            "needs",
            "repo",
            "url",
            "reason",
            "catalog_override",
        ]
        .contains(&k.as_str())
        {
            p.push(format!("{label}: unknown key '{k}'"));
        }
    }
    let src = o.get("source").and_then(Value::as_str).unwrap_or("");
    if !allowed_sources(target).contains(&src) {
        p.push(format!("{label}: source '{src}' not allowed for {target}"));
        return p;
    }
    match o.get("confidence").and_then(Value::as_str) {
        Some(c) if CONFIDENCE.contains(&c) => {}
        other => p.push(format!("{label}: bad confidence {other:?}")),
    }
    let pkg = o.get("pkg").and_then(Value::as_str);
    match src {
        "none" => {
            if o.get("reason")
                .and_then(Value::as_str)
                .map_or(true, |r| r.trim().len() < 10)
            {
                p.push(format!(
                    "{label}: source none needs a 'reason' (>= 10 chars)"
                ));
            }
            if pkg.is_some() {
                p.push(format!("{label}: source none must not have pkg"));
            }
        }
        "download" => {
            match o.get("url").and_then(Value::as_str) {
                Some(u) if u.starts_with("https://") && !u.contains(' ') => {}
                other => p.push(format!(
                    "{label}: download needs an https url, got {other:?}"
                )),
            }
            if pkg.is_some() {
                p.push(format!("{label}: download must not have pkg"));
            }
        }
        "flatpak" => match pkg {
            Some(x) if is_flatpak_id(x) => {}
            other => p.push(format!("{label}: bad flatpak id {other:?}")),
        },
        _ => match pkg {
            Some(x) if is_pkg_name(x) => {}
            other => p.push(format!("{label}: bad package name {other:?}")),
        },
    }
    if matches!(src, "copr" | "ppa" | "obs") {
        let r = o.get("repo").and_then(Value::as_str).unwrap_or("");
        let ok = match src {
            "copr" => {
                r.split('/').count() == 2 && r.split('/').all(|s| !s.is_empty() && !s.contains(' '))
            }
            "ppa" => r.starts_with("ppa:") && r.matches('/').count() == 1,
            _ => r.starts_with("https://"),
        };
        if !ok {
            p.push(format!(
                "{label}: {src} needs a well-formed 'repo', got '{r}'"
            ));
        }
    } else if o.contains_key("repo") {
        p.push(format!("{label}: 'repo' only valid for copr/ppa/obs"));
    }
    if let Some(n) = o.get("needs") {
        let known: BTreeSet<&str> = targets["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == target)
            .unwrap()["repos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        match n.as_array() {
            Some(a)
                if !a.is_empty()
                    && a.iter()
                        .all(|x| x.as_str().map_or(false, |s| known.contains(s))) => {}
            _ => p.push(format!(
                "{label}: 'needs' must be a non-empty list of repo ids defined for {target}"
            )),
        }
    }
    p
}

fn targets_doc() -> Value {
    read_json(&root().join("distros/targets.json"))
}

#[test]
fn targets_file_is_well_formed() {
    let t = targets_doc();
    assert_eq!(t["schema_version"], 1);
    let ids: Vec<&str> = t["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, TARGETS, "target list/order");
    for x in t["targets"].as_array().unwrap() {
        let id = x["id"].as_str().unwrap();
        for k in [
            "name",
            "family",
            "package_manager",
            "install_cmd",
            "refresh_cmd",
        ] {
            assert!(
                x[k].as_str().map_or(false, |s| !s.is_empty()),
                "{id}: missing {k}"
            );
        }
        assert!(
            x["install_cmd"].as_str().unwrap().contains("{pkgs}"),
            "{id}: install_cmd needs {{pkgs}}"
        );
        assert!(x["flatpak_preinstalled"].is_boolean());
        let repos = x["repos"].as_array().unwrap();
        assert!(!repos.is_empty());
        let mut seen = BTreeSet::new();
        for r in repos {
            assert!(
                seen.insert(r["id"].as_str().unwrap()),
                "{id}: duplicate repo id"
            );
            for k in ["name", "enable", "note"] {
                assert!(
                    r[k].as_str().map_or(false, |s| !s.is_empty()),
                    "{id}/{}: missing {k}",
                    r["id"]
                );
            }
        }
    }
}

#[test]
fn post_install_notes_are_well_formed() {
    let t = targets_doc();
    let cat = catalog();
    let cats: BTreeSet<&str> = cat.apps.values().map(|(c, _)| c.as_str()).collect();
    let mut all_ids = BTreeSet::new();
    for tg in TARGETS {
        let notes = t["post_install_notes"][tg]
            .as_array()
            .unwrap_or_else(|| panic!("no notes for {tg}"));
        assert!(notes.len() >= 3, "{tg}: too few notes");
        for n in notes {
            let id = n["id"].as_str().unwrap();
            assert!(
                id.starts_with(&format!("{tg}-")),
                "{id} must be prefixed with {tg}-"
            );
            assert!(all_ids.insert(id.to_string()), "duplicate note id {id}");
            let text = n["text"].as_str().unwrap();
            assert!(
                text.len() >= 20 && text.ends_with('.'),
                "{id}: note text should be a sentence"
            );
            let when = n["when"].as_str().unwrap();
            if let Some(c) = when.strip_prefix("category:") {
                assert!(cats.contains(c), "{id}: unknown category {c}");
            } else if let Some(a) = when.strip_prefix("app:") {
                assert!(cat.apps.contains_key(a), "{id}: unknown app {a}");
            } else {
                assert_eq!(when, "always", "{id}: bad 'when'");
            }
        }
    }
    let known: BTreeSet<&str> = t["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    for k in t["post_install_notes"].as_object().unwrap().keys() {
        assert!(known.contains(k.as_str()), "notes for unknown target {k}");
    }
}

#[test]
fn shards_parse_without_duplicates() {
    assert!(
        !shard_files("packages").is_empty(),
        "no distros/packages/*.json shards"
    );
    let (_, errs) = load_shards("packages");
    assert!(errs.is_empty(), "{}", errs.join("\n"));
}

#[test]
fn every_catalog_app_with_linux_packages_is_mapped_for_every_target() {
    let cat = catalog();
    let (map, _) = load_shards("packages");
    let want: BTreeSet<&String> = cat
        .apps
        .iter()
        .filter(|(_, (_, l))| !l.is_null())
        .map(|(i, _)| i)
        .collect();
    let have: BTreeSet<&String> = map.keys().collect();
    let missing: Vec<_> = want.difference(&have).collect();
    let extra: Vec<_> = have.difference(&want).collect();
    assert!(
        missing.is_empty(),
        "{} unmapped catalog apps: {missing:?}",
        missing.len()
    );
    assert!(
        extra.is_empty(),
        "mapped ids that are not native catalog apps: {extra:?}"
    );
    let t = targets_doc();
    let mut problems = vec![];
    for (id, e) in &map {
        let o = e
            .as_object()
            .unwrap_or_else(|| panic!("{id}: entry must be an object"));
        for k in o.keys() {
            if !TARGETS.contains(&k.as_str()) && k != "notes" {
                problems.push(format!("{id}: unknown key '{k}'"));
            }
        }
        for tg in TARGETS {
            match o.get(tg) {
                None => problems.push(format!("{id}: missing target {tg}")),
                Some(x) => problems.extend(check_entry(&format!("{id}/{tg}"), tg, x, &t)),
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn mappings_agree_with_the_catalog() {
    let cat = catalog();
    let (map, _) = load_shards("packages");
    let mut problems = vec![];
    for (id, e) in &map {
        let Some((_, linux)) = cat.apps.get(id) else {
            continue;
        };
        let cat_apt = linux["apt"].as_str();
        let cat_dnf = linux["dnf"].as_str();
        let cat_fp = linux["flatpak"].as_str();
        for (tg, cat_native) in [
            ("fedora", cat_dnf),
            ("debian", cat_apt),
            ("ubuntu", cat_apt),
        ] {
            let x = &e[tg];
            let src = x["source"].as_str().unwrap_or("");
            if let Some(n) = cat_native {
                if src == "repo"
                    && x["pkg"].as_str() != Some(n)
                    && x.get("catalog_override").is_none()
                {
                    problems.push(format!(
                        "{id}/{tg}: pkg {:?} differs from catalog '{n}' without catalog_override",
                        x["pkg"]
                    ));
                }
                if src == "none" && cat_fp.is_none() {
                    problems.push(format!(
                        "{id}/{tg}: catalog lists '{n}' but mapping says none"
                    ));
                }
            }
        }
        for tg in TARGETS {
            let x = &e[tg];
            let src = x["source"].as_str().unwrap_or("");
            if src == "flatpak" {
                if let Some(fp) = cat_fp {
                    if x["pkg"].as_str() != Some(fp) {
                        problems.push(format!(
                            "{id}/{tg}: flatpak id {:?} differs from catalog '{fp}'",
                            x["pkg"]
                        ));
                    }
                }
            }
            if src == "none" && cat_fp.is_some() {
                problems.push(format!(
                    "{id}/{tg}: catalog has flatpak '{}' but mapping says none",
                    cat_fp.unwrap()
                ));
            }
            if x.get("catalog_override").is_some()
                && x["catalog_override"]
                    .as_str()
                    .map_or(true, |s| s.len() < 10)
            {
                problems.push(format!("{id}/{tg}: catalog_override needs a reason"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn every_app_is_installable_somewhere_and_notes_are_sane() {
    let (map, _) = load_shards("packages");
    let mut problems = vec![];
    for (id, e) in &map {
        let usable = TARGETS
            .iter()
            .filter(|t| e[**t]["source"] != "none")
            .count();
        if usable == 0 {
            problems.push(format!("{id}: no installable target at all"));
        }
        if let Some(n) = e.get("notes") {
            match n.as_array() {
                Some(a)
                    if a.iter().all(|s| {
                        s.as_str()
                            .map_or(false, |s| s.len() >= 15 && s.ends_with('.'))
                    }) => {}
                _ => problems.push(format!("{id}: notes must be sentences ending with '.'")),
            }
        }
        // repo-built packages that need a third-party repo must say so
        for tg in TARGETS {
            let x = &e[tg];
            let src = x["source"].as_str().unwrap_or("");
            let needs_repo = matches!(src, "rpmfusion" | "packman" | "aur" | "vendor-repo");
            if needs_repo && x.get("needs").is_none() && !matches!(src, "aur") {
                problems.push(format!("{id}/{tg}: source {src} should list 'needs'"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn alternatives_shards_are_valid_if_present() {
    let files = shard_files("alternatives");
    if files.is_empty() {
        return;
    }
    let cat = catalog();
    let (map, errs) = load_shards("alternatives");
    assert!(errs.is_empty(), "{}", errs.join("\n"));
    let t = targets_doc();
    let mut problems = vec![];
    for (key, e) in &map {
        // key: "<app id>::<alternative name>"
        let Some((id, name)) = key.split_once("::") else {
            problems.push(format!("{key}: key must be '<app id>::<alternative name>'"));
            continue;
        };
        match cat.alts.get(id) {
            Some(l) if l.iter().any(|(n, _)| n == name) => {
                let fp = l
                    .iter()
                    .find(|(n, _)| n == name)
                    .and_then(|(_, f)| f.clone());
                for tg in TARGETS {
                    match e.get(tg) {
                        None => problems.push(format!("{key}: missing target {tg}")),
                        Some(x) => {
                            problems.extend(check_entry(&format!("{key}/{tg}"), tg, x, &t));
                            if x["source"] == "flatpak" {
                                if let Some(f) = &fp {
                                    if x["pkg"].as_str() != Some(f.as_str()) {
                                        problems.push(format!(
                                            "{key}/{tg}: flatpak id differs from catalog '{f}'"
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => problems.push(format!("{key}: not an alternative in data/app_map.json")),
        }
    }
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn coverage_report() {
    // Not an assertion on content: prints coverage so `cargo test -- --nocapture` shows it.
    let cat = catalog();
    let (map, _) = load_shards("packages");
    let (amap, _) = load_shards("alternatives");
    let total_alts: usize = cat.alts.values().map(|l| l.len()).sum();
    let mut by_src: BTreeMap<String, usize> = BTreeMap::new();
    for e in map.values().chain(amap.values()) {
        for tg in TARGETS {
            if let Some(s) = e[tg]["source"].as_str() {
                *by_src.entry(format!("{tg}:{s}")).or_default() += 1;
            }
        }
    }
    println!(
        "native apps mapped: {}; alternatives mapped: {}/{}",
        map.len(),
        amap.len(),
        total_alts
    );
    println!("sources: {by_src:?}");
}

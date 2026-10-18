//! Maps installed Windows programs to their Linux story using data/app_map.json.
//! Pure logic; see data/SCHEMA.md for the file format and matching rules.

use crate::scan::programs::{normalize_name, Program};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const EMBEDDED_APP_MAP: &str = include_str!("../data/app_map.json");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Blocked,
    Alternative,
    WineOk,
    Native,
    Unknown,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Native => "Has a Linux version",
            Status::Alternative => "Use a Linux alternative",
            Status::WineOk => "Runs via Wine/Proton",
            Status::Blocked => "Won't work on Linux",
            Status::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LinuxPackages {
    pub flatpak: Option<String>,
    pub apt: Option<String>,
    pub dnf: Option<String>,
    pub snap: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Alternative {
    pub name: String,
    pub flatpak: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    #[serde(rename = "match")]
    pub patterns: Vec<String>,
    pub category: String,
    pub status: Status,
    #[serde(default)]
    pub linux: Option<LinuxPackages>,
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IgnoreRule {
    #[serde(rename = "match")]
    pub patterns: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppMap {
    pub schema_version: u32,
    #[serde(default)]
    pub ignore: Vec<IgnoreRule>,
    pub apps: Vec<AppEntry>,
}

/// Returns the length of the pattern's text if it matches `norm`.
pub fn pattern_match_len(pattern: &str, norm: &str) -> Option<usize> {
    if let Some(p) = pattern.strip_prefix('=') {
        (norm == p).then_some(p.len())
    } else if let Some(p) = pattern.strip_prefix('^') {
        norm.starts_with(p).then_some(p.len())
    } else {
        (!pattern.is_empty() && norm.contains(pattern)).then_some(pattern.len())
    }
}

fn best_len(patterns: &[String], norm: &str) -> Option<usize> {
    patterns.iter().filter_map(|p| pattern_match_len(p, norm)).max()
}

impl AppMap {
    pub fn parse(json: &str) -> Result<AppMap, String> {
        let map: AppMap = serde_json::from_str(json).map_err(|e| format!("app_map.json is not valid: {e}"))?;
        let problems = map.validate();
        if problems.is_empty() {
            Ok(map)
        } else {
            Err(format!("app_map.json has problems:\n  - {}", problems.join("\n  - ")))
        }
    }

    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut ids = HashSet::new();
        for a in &self.apps {
            if !ids.insert(a.id.as_str()) {
                problems.push(format!("duplicate id '{}'", a.id));
            }
            if a.patterns.is_empty() {
                problems.push(format!("'{}' has no match patterns", a.id));
            }
            for p in &a.patterns {
                if p.trim_start_matches(['^', '=']).is_empty() {
                    problems.push(format!("'{}' has an empty pattern", a.id));
                }
                if *p != p.to_lowercase() {
                    problems.push(format!("'{}' pattern '{p}' must be lowercase", a.id));
                }
            }
            if a.status == Status::Alternative && a.alternatives.is_empty() {
                problems.push(format!("'{}' is 'alternative' but lists no alternatives", a.id));
            }
        }
        for r in &self.ignore {
            for p in &r.patterns {
                if *p != p.to_lowercase() || p.trim_start_matches(['^', '=']).is_empty() {
                    problems.push(format!("ignore pattern '{p}' must be lowercase and non-empty"));
                }
            }
        }
        problems
    }

    /// Loads the first app_map.json found: `--data-dir`, then `data/` next to
    /// the exe, then the copy compiled into the exe.
    pub fn load(data_dir: Option<&Path>) -> Result<(AppMap, String), String> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(d) = data_dir {
            candidates.push(d.join("app_map.json"));
        }
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
            candidates.push(exe_dir.join("data").join("app_map.json"));
        }
        for c in candidates {
            if let Ok(text) = std::fs::read_to_string(&c) {
                return Ok((AppMap::parse(&text)?, c.display().to_string()));
            }
        }
        if data_dir.is_some() {
            return Err("no app_map.json in the given --data-dir".into());
        }
        Ok((AppMap::parse(EMBEDDED_APP_MAP)?, "built-in copy".into()))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// Found in app_map.json.
    Mapped,
    /// A runtime/driver that doesn't matter on Linux.
    Ignored,
    /// Microsoft-signed Store package not in the map (Calculator, Photos...).
    WindowsBuiltin,
    /// Not in app_map.json at all.
    Unmapped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappedProgram {
    pub program: Program,
    pub group: Group,
    pub status: Status,
    pub entry: Option<AppEntry>,
    pub ignore_reason: Option<String>,
    /// ProtonDB page for Steam games (built from the real Steam app id).
    pub protondb: Option<String>,
}

pub fn classify(p: &Program, map: &AppMap) -> MappedProgram {
    let norm = normalize_name(&p.name);
    let best_app = map
        .apps
        .iter()
        .filter_map(|a| best_len(&a.patterns, &norm).map(|l| (l, a)))
        .max_by_key(|(l, _)| *l);
    let best_ignore = map
        .ignore
        .iter()
        .filter_map(|r| best_len(&r.patterns, &norm).map(|l| (l, r)))
        .max_by_key(|(l, _)| *l);
    let protondb = p.steam_appid.map(|id| format!("https://www.protondb.com/app/{id}"));

    let (group, status, entry, ignore_reason) = match (best_app, best_ignore) {
        (Some((la, a)), Some((li, _))) if la >= li => (Group::Mapped, a.status, Some(a.clone()), None),
        (Some((_, a)), None) => (Group::Mapped, a.status, Some(a.clone()), None),
        (_, Some((_, r))) => (Group::Ignored, Status::Unknown, None, Some(r.reason.clone())),
        (None, None) if p.microsoft_msix => (Group::WindowsBuiltin, Status::Unknown, None, None),
        (None, None) => (Group::Unmapped, Status::Unknown, None, None),
    };
    MappedProgram { program: p.clone(), group, status, entry, ignore_reason, protondb }
}

pub fn classify_all(programs: &[Program], map: &AppMap) -> Vec<MappedProgram> {
    programs.iter().map(|p| classify(p, map)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(name: &str) -> Program {
        Program { name: name.into(), publisher: None, version: None, sources: vec![], winget_id: None, steam_appid: None, microsoft_msix: false }
    }

    fn map() -> AppMap {
        AppMap::parse(
            r#"{"schema_version":1,
            "ignore":[{"match":["microsoft visual c++"],"reason":"runtime"}],
            "apps":[
              {"id":"vs","name":"Visual Studio","match":["^visual studio","microsoft visual studio"],"category":"dev","status":"alternative","alternatives":[{"name":"VS Code"}]},
              {"id":"vscode","name":"VS Code","match":["microsoft visual studio code"],"category":"dev","status":"native"},
              {"id":"steam","name":"Steam","match":["=steam"],"category":"game-launcher","status":"native"},
              {"id":"fortnite","name":"Fortnite","match":["=fortnite"],"category":"game","status":"blocked"}
            ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn pattern_kinds() {
        assert_eq!(pattern_match_len("steam", "steamvr"), Some(5));
        assert_eq!(pattern_match_len("=steam", "steamvr"), None);
        assert_eq!(pattern_match_len("=steam", "steam"), Some(5));
        assert_eq!(pattern_match_len("^git", "github cli"), Some(3));
        assert_eq!(pattern_match_len("^git", "the git"), None);
        assert_eq!(pattern_match_len("", "x"), None);
    }

    #[test]
    fn longest_pattern_wins() {
        let m = map();
        assert_eq!(classify(&prog("Microsoft Visual Studio Code (User)"), &m).entry.unwrap().id, "vscode");
        assert_eq!(classify(&prog("Visual Studio Community 2022"), &m).entry.unwrap().id, "vs");
    }

    #[test]
    fn exact_match_does_not_catch_neighbours() {
        let m = map();
        assert_eq!(classify(&prog("Steam"), &m).status, Status::Native);
        assert_eq!(classify(&prog("SteamVR"), &m).group, Group::Unmapped);
    }

    #[test]
    fn ignored_runtimes_and_builtins() {
        let m = map();
        let r = classify(&prog("Microsoft Visual C++ 2013 Redistributable (x64)"), &m);
        assert_eq!(r.group, Group::Ignored);
        let mut calc = prog("Windows Calculator");
        calc.microsoft_msix = true;
        assert_eq!(classify(&calc, &m).group, Group::WindowsBuiltin);
        // A Microsoft Store package that *is* in the map is still reported.
        let mut fort = prog("Fortnite");
        fort.microsoft_msix = true;
        assert_eq!(classify(&fort, &m).status, Status::Blocked);
    }

    #[test]
    fn steam_games_get_protondb_link() {
        let mut p = prog("Among Us");
        p.steam_appid = Some(945360);
        let r = classify(&p, &map());
        assert_eq!(r.protondb.as_deref(), Some("https://www.protondb.com/app/945360"));
        assert_eq!(r.status, Status::Unknown);
    }

    #[test]
    fn validation_catches_mistakes() {
        let bad = r#"{"schema_version":1,"apps":[
            {"id":"a","name":"A","match":["Upper"],"category":"x","status":"native"},
            {"id":"a","name":"A2","match":[],"category":"x","status":"alternative"}]}"#;
        let err = AppMap::parse(bad).unwrap_err();
        assert!(err.contains("duplicate id"));
        assert!(err.contains("lowercase"));
        assert!(err.contains("no match patterns"));
        assert!(err.contains("lists no alternatives"));
    }

    #[test]
    fn embedded_app_map_is_valid() {
        let m = AppMap::parse(EMBEDDED_APP_MAP).expect("data/app_map.json must parse and validate");
        assert!(m.apps.len() >= 150, "need at least 150 apps, have {}", m.apps.len());
    }
}

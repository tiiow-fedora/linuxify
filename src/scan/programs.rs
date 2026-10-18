//! Installed programs: Windows "Add/Remove Programs" registry keys plus `winget list`.
//!
//! The OS-touching parts (`read_registry`, `run_winget`) are thin; all the
//! filtering, parsing and de-duplication is pure and unit-tested.

use serde::{Deserialize, Serialize};

/// One raw entry under an `...\CurrentVersion\Uninstall\<key>` registry key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawUninstallEntry {
    pub key_name: String,
    /// "HKLM64", "HKLM32" or "HKCU"
    pub hive: String,
    pub display_name: Option<String>,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub system_component: bool,
    pub parent_key_name: Option<String>,
    pub release_type: Option<String>,
}

/// One row of `winget list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WingetRow {
    pub name: String,
    pub id: String,
    pub version: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Program {
    pub name: String,
    pub publisher: Option<String>,
    pub version: Option<String>,
    /// Where we saw it: "HKLM64", "HKLM32", "HKCU", "winget".
    pub sources: Vec<String>,
    pub winget_id: Option<String>,
    pub steam_appid: Option<u32>,
    /// A Microsoft-signed Store/MSIX package (mostly Windows' own apps).
    pub microsoft_msix: bool,
}

/// Lowercase, drop ™ ® © and control characters, collapse whitespace, trim.
/// This is the form `app_map.json` patterns are matched against.
///
/// Control characters matter: real registry names contain NUL padding and
/// raw Windows-1252 bytes (0x99 is ™ there but a C1 control in Unicode).
pub fn normalize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '™' | '®' | '©') && !c.is_control())
        .collect::<String>()
        .to_lowercase();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Display form of a name: control characters removed (a stray 0x99 becomes ™), trimmed.
fn clean_display(name: &str) -> String {
    name.chars()
        .filter_map(|c| match c {
            '\u{99}' => Some('™'),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Registry keys named "Steam App 945360" are Steam games.
pub fn steam_appid_from_key(key_name: &str) -> Option<u32> {
    key_name.strip_prefix("Steam App ")?.trim().parse().ok()
}

/// Drops entries Windows itself hides from "Add/Remove Programs":
/// no name, SystemComponent=1, child updates (ParentKeyName), hotfixes.
pub fn is_user_visible(e: &RawUninstallEntry) -> bool {
    let has_name = e.display_name.as_deref().is_some_and(|n| !n.trim().is_empty());
    let is_update = matches!(
        e.release_type.as_deref(),
        Some("Update") | Some("Hotfix") | Some("Security Update") | Some("ServicePack")
    );
    has_name && !e.system_component && e.parent_key_name.is_none() && !is_update
}

/// Parses the fixed-width table printed by `winget list`.
///
/// Column boundaries come from where each header word starts. Lines before
/// the header (progress spinners) are ignored; `\r` progress overwrites are
/// resolved by keeping only the text after the last `\r`.
pub fn parse_winget_table(output: &str) -> Vec<WingetRow> {
    let lines: Vec<String> = output
        .split('\n')
        .map(|l| l.trim_end_matches('\r').rsplit('\r').next().unwrap_or("").trim_end().to_string())
        .collect();

    // The header is the line directly above a line made only of dashes.
    let Some(sep_idx) = lines
        .iter()
        .position(|l| l.len() >= 10 && l.chars().all(|c| c == '-'))
    else {
        return Vec::new();
    };
    if sep_idx == 0 {
        return Vec::new();
    }
    let header: Vec<char> = lines[sep_idx - 1].chars().collect();

    // Start column of each header word.
    let mut starts = Vec::new();
    let mut titles = Vec::new();
    let mut i = 0;
    while i < header.len() {
        if header[i] != ' ' && (i == 0 || header[i - 1] == ' ') {
            starts.push(i);
            let end = (i..header.len()).find(|&j| header[j] == ' ').unwrap_or(header.len());
            titles.push(header[i..end].iter().collect::<String>().to_lowercase());
        }
        i += 1;
    }
    if starts.len() < 3 {
        return Vec::new();
    }
    // Find columns by English title when possible, otherwise by position.
    let col = |title: &str, fallback: Option<usize>| -> Option<usize> {
        titles.iter().position(|t| t == title).or(fallback)
    };
    let name_c = col("name", Some(0)).unwrap();
    let id_c = col("id", Some(1)).unwrap();
    let ver_c = col("version", Some(2)).unwrap();
    let src_c = col("source", if starts.len() >= 4 { Some(starts.len() - 1) } else { None });

    let cell = |row: &[char], c: usize| -> String {
        let from = starts[c];
        let to = starts.get(c + 1).copied().unwrap_or(usize::MAX).min(row.len());
        if from >= row.len() || from >= to {
            return String::new();
        }
        row[from..to].iter().collect::<String>().trim().to_string()
    };

    lines[sep_idx + 1..]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().collect::<Vec<char>>())
        .filter_map(|row| {
            let name = cell(&row, name_c);
            let id = cell(&row, id_c);
            if name.is_empty() || id.is_empty() {
                return None;
            }
            Some(WingetRow {
                name,
                id,
                version: cell(&row, ver_c),
                source: src_c.map(|c| cell(&row, c)).unwrap_or_default(),
            })
        })
        .collect()
}

/// MSIX packages signed by Microsoft carry this publisher hash.
const MICROSOFT_PUBLISHER_HASH: &str = "8wekyb3d8bbwe";

pub fn is_microsoft_msix(winget_id: &str) -> bool {
    winget_id.starts_with("MSIX\\") && winget_id.ends_with(MICROSOFT_PUBLISHER_HASH)
}

/// Merges registry entries and winget rows into one de-duplicated list.
///
/// Two records are the same program when their normalised names are equal.
/// (Versions are deliberately not stripped from names: "Portal" and
/// "Portal 2" are different programs.)
pub fn merge_programs(registry: &[RawUninstallEntry], winget: &[WingetRow]) -> Vec<Program> {
    let mut out: Vec<Program> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = Default::default();

    for e in registry.iter().filter(|e| is_user_visible(e)) {
        let name = clean_display(e.display_name.as_deref().unwrap_or_default());
        let key = normalize_name(&name);
        if let Some(&i) = index.get(&key) {
            let p = &mut out[i];
            if !p.sources.contains(&e.hive) {
                p.sources.push(e.hive.clone());
            }
            p.publisher = p.publisher.take().or_else(|| e.publisher.clone());
            p.version = p.version.take().or_else(|| e.version.clone());
            p.steam_appid = p.steam_appid.or_else(|| steam_appid_from_key(&e.key_name));
            continue;
        }
        index.insert(key, out.len());
        out.push(Program {
            name,
            publisher: e.publisher.clone().filter(|s| !s.trim().is_empty()),
            version: e.version.clone().filter(|s| !s.trim().is_empty()),
            sources: vec![e.hive.clone()],
            winget_id: None,
            steam_appid: steam_appid_from_key(&e.key_name),
            microsoft_msix: false,
        });
    }

    for w in winget {
        let key = normalize_name(&w.name);
        let version = Some(w.version.clone()).filter(|v| !v.is_empty() && v != "Unknown");
        // Prefer a real package id (e.g. "7zip.7zip") over winget's ARP/MSIX pseudo ids.
        let real_id = !(w.id.starts_with("ARP\\") || w.id.starts_with("MSIX\\"));
        if let Some(&i) = index.get(&key) {
            let p = &mut out[i];
            if !p.sources.iter().any(|s| s == "winget") {
                p.sources.push("winget".into());
            }
            if p.winget_id.is_none() || real_id {
                p.winget_id = Some(w.id.clone());
            }
            p.version = p.version.take().or(version);
            continue;
        }
        index.insert(key, out.len());
        out.push(Program {
            name: w.name.clone(),
            publisher: None,
            version,
            sources: vec!["winget".into()],
            winget_id: Some(w.id.clone()),
            steam_appid: w
                .id
                .rsplit('\\')
                .next()
                .and_then(steam_appid_from_key),
            microsoft_msix: is_microsoft_msix(&w.id),
        });
    }

    out.sort_by_key(|p| normalize_name(&p.name));
    out
}

// ---------------------------------------------------------------------------
// OS-touching code (kept thin)
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub fn read_registry() -> Vec<RawUninstallEntry> {
    use winreg::enums::*;
    use winreg::RegKey;

    const PATH: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let sources = [
        (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY, "HKLM64"),
        (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY, "HKLM32"),
        (HKEY_CURRENT_USER, 0, "HKCU"),
    ];
    let mut out = Vec::new();
    for (root, wow, hive) in sources {
        let Ok(key) = RegKey::predef(root).open_subkey_with_flags(PATH, KEY_READ | wow) else {
            continue;
        };
        for name in key.enum_keys().flatten() {
            let Ok(sub) = key.open_subkey_with_flags(&name, KEY_READ | wow) else {
                continue;
            };
            let s = |v: &str| sub.get_value::<String, _>(v).ok();
            out.push(RawUninstallEntry {
                key_name: name.clone(),
                hive: hive.to_string(),
                display_name: s("DisplayName"),
                publisher: s("Publisher"),
                version: s("DisplayVersion"),
                system_component: sub.get_value::<u32, _>("SystemComponent").unwrap_or(0) == 1,
                parent_key_name: s("ParentKeyName"),
                release_type: s("ReleaseType"),
            });
        }
    }
    out
}

#[cfg(not(windows))]
pub fn read_registry() -> Vec<RawUninstallEntry> {
    Vec::new()
}

/// Runs `winget list`. Returns None if winget is missing or fails; the scan
/// carries on with registry data only. `--disable-interactivity` means we never
/// answer (or accept) any prompt on the user's behalf.
pub fn run_winget() -> Option<String> {
    let out = std::process::Command::new("winget")
        .args(["list", "--disable-interactivity"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, hive: &str) -> RawUninstallEntry {
        RawUninstallEntry {
            key_name: name.into(),
            hive: hive.into(),
            display_name: Some(name.into()),
            ..Default::default()
        }
    }

    #[test]
    fn normalize_strips_marks_and_spaces() {
        assert_eq!(normalize_name("  HWiNFO®  64 "), "hwinfo 64");
        assert_eq!(normalize_name("HELLDIVERS™ 2"), "helldivers 2");
        assert_eq!(normalize_name("Roblox Player for Bob  "), "roblox player for bob");
        // Seen on a real machine: NUL padding and a raw cp1252 ™ byte.
        assert_eq!(normalize_name("Roblox Player for Bob \u{0}"), "roblox player for bob");
        assert_eq!(normalize_name("HELLDIVERS\u{99} 2"), "helldivers 2");
    }

    #[test]
    fn hidden_entries_are_dropped() {
        let mut sys = entry("Some Driver", "HKLM64");
        sys.system_component = true;
        let mut upd = entry("Update for X", "HKLM64");
        upd.parent_key_name = Some("X".into());
        let mut hotfix = entry("KB123", "HKLM64");
        hotfix.release_type = Some("Hotfix".into());
        let mut blank = entry("x", "HKLM64");
        blank.display_name = Some("   ".into());
        for e in [&sys, &upd, &hotfix, &blank] {
            assert!(!is_user_visible(e), "{e:?}");
        }
        assert!(is_user_visible(&entry("7-Zip", "HKLM64")));
    }

    #[test]
    fn steam_ids() {
        assert_eq!(steam_appid_from_key("Steam App 945360"), Some(945360));
        assert_eq!(steam_appid_from_key("Steam"), None);
        assert_eq!(steam_appid_from_key("Steam App abc"), None);
    }

    #[test]
    fn dedupe_across_hives_but_not_sequels() {
        let reg = vec![
            entry("Portal", "HKLM64"),
            entry("Portal 2", "HKLM64"),
            entry("Git", "HKLM64"),
            entry("Git", "HKLM32"),
        ];
        let progs = merge_programs(&reg, &[]);
        assert_eq!(progs.len(), 3);
        let git = progs.iter().find(|p| p.name == "Git").unwrap();
        assert_eq!(git.sources, vec!["HKLM64", "HKLM32"]);
    }

    /// Builds a table laid out the way winget pads it (columns 20/60/14/12).
    fn winget_sample() -> String {
        let row = |a: &str, b: &str, c: &str, d: &str, e: &str| {
            format!("{a:<20}{b:<60}{c:<14}{d:<12}{e}\n")
        };
        let mut s = String::from("\r   - \r   \\ \r");
        s += &row("Name", "Id", "Version", "Available", "Source");
        s += &"-".repeat(110);
        s += "\n";
        s += &row("7-Zip 25.01 (x64)", "7zip.7zip", "25.01", "26.04", "winget");
        s += &row("Among Us", "ARP\\Machine\\X64\\Steam App 945360", "Unknown", "", "");
        s += &row("Fortnite", "MSIX\\436609B6.FortniteClient_1.5_x64__9ncxwbgmmv7m8", "1.5", "", "");
        s += &row("Game Bar", "MSIX\\Microsoft.XboxGamingOverlay_7.3_x64__8wekyb3d8bbwe", "7.3", "", "");
        s
    }

    #[test]
    fn parses_winget_table() {
        let rows = parse_winget_table(&winget_sample());
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].name, "7-Zip 25.01 (x64)");
        assert_eq!(rows[0].id, "7zip.7zip");
        assert_eq!(rows[0].version, "25.01");
        assert_eq!(rows[0].source, "winget");
        assert_eq!(rows[1].id, "ARP\\Machine\\X64\\Steam App 945360");
        assert_eq!(rows[1].source, "");
    }

    #[test]
    fn parses_crlf_output() {
        // Real winget output, captured through a pipe, uses CRLF line endings.
        let rows = parse_winget_table(&winget_sample().replace('\n', "\r\n"));
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[3].name, "Game Bar");
    }

    #[test]
    fn parses_nothing_from_garbage() {
        assert!(parse_winget_table("No installed package found.").is_empty());
        assert!(parse_winget_table("").is_empty());
    }

    #[test]
    fn winget_merges_into_registry_and_adds_store_apps() {
        let reg = vec![entry("7-Zip 25.01 (x64)", "HKLM64"), entry("Among Us", "HKLM64")];
        let rows = vec![
            WingetRow { name: "7-Zip 25.01 (x64)".into(), id: "7zip.7zip".into(), version: "25.01".into(), source: "winget".into() },
            WingetRow { name: "Among Us".into(), id: "ARP\\Machine\\X64\\Steam App 945360".into(), version: "Unknown".into(), source: "".into() },
            WingetRow { name: "Game Bar".into(), id: "MSIX\\Microsoft.XboxGamingOverlay_7.3_x64__8wekyb3d8bbwe".into(), version: "7.3".into(), source: "".into() },
        ];
        let progs = merge_programs(&reg, &rows);
        assert_eq!(progs.len(), 3);
        let zip = progs.iter().find(|p| p.name.starts_with("7-Zip")).unwrap();
        assert_eq!(zip.winget_id.as_deref(), Some("7zip.7zip"));
        assert_eq!(zip.sources, vec!["HKLM64", "winget"]);
        let bar = progs.iter().find(|p| p.name == "Game Bar").unwrap();
        assert!(bar.microsoft_msix);
        assert_eq!(bar.version.as_deref(), Some("7.3"));
    }

    #[test]
    fn winget_only_steam_game_gets_appid() {
        let rows = vec![WingetRow {
            name: "Portal".into(),
            id: "ARP\\Machine\\X64\\Steam App 400".into(),
            version: "Unknown".into(),
            source: "".into(),
        }];
        let progs = merge_programs(&[], &rows);
        assert_eq!(progs[0].steam_appid, Some(400));
        assert_eq!(progs[0].version, None);
    }
}

//! User data inventory: library folders, browser/mail profiles, password
//! databases, Steam libraries and app data worth knowing about.
//!
//! Everything here only *reads* metadata (sizes, names). Nothing is opened for
//! writing and no file contents are read except small config files
//! (profiles.ini, Local State, libraryfolders.vdf, appmanifest_*.acf).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FolderStats {
    pub bytes: u64,
    pub files: u64,
    /// OneDrive/cloud placeholders: listed on disk but contents live online.
    pub cloud_only_files: u64,
    pub cloud_only_bytes: u64,
    /// Junctions/symlinks we deliberately did not follow.
    pub links_skipped: u64,
    /// Folders/files we could not read (permissions, in use).
    pub unreadable: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryFolder {
    pub label: String,
    pub path: String,
    pub exists: bool,
    pub stats: FolderStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserProfile {
    pub app: String,
    pub profile_name: String,
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamLibrary {
    pub path: String,
    pub games: Vec<SteamGame>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SteamGame {
    pub appid: u32,
    pub name: String,
    pub size_on_disk: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDataItem {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserData {
    pub folders: Vec<LibraryFolder>,
    pub browsers: Vec<BrowserProfile>,
    pub mail: Vec<BrowserProfile>,
    pub keepass_files: Vec<String>,
    pub outlook_pst_files: Vec<String>,
    pub steam: Vec<SteamLibrary>,
    pub app_data: Vec<AppDataItem>,
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Expands %VAR% references using `lookup`. Unknown variables are left as-is.
pub fn expand_env(s: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match lookup(name) {
                    Some(v) if !name.is_empty() => out.push_str(&v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A profile entry from a Mozilla profiles.ini.
#[derive(Debug, Clone, PartialEq)]
pub struct IniProfile {
    pub name: String,
    pub path: String,
    pub is_relative: bool,
}

/// Parses Firefox/Thunderbird/LibreWolf `profiles.ini`.
pub fn parse_profiles_ini(text: &str) -> Vec<IniProfile> {
    let mut out = Vec::new();
    let mut cur: Option<IniProfile> = None;
    let mut in_profile = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            if let Some(p) = cur.take() {
                if !p.path.is_empty() {
                    out.push(p);
                }
            }
            in_profile = line.to_ascii_lowercase().starts_with("[profile");
            if in_profile {
                cur = Some(IniProfile { name: String::new(), path: String::new(), is_relative: true });
            }
            continue;
        }
        let (Some(p), true) = (cur.as_mut(), in_profile) else { continue };
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "Name" => p.name = v.trim().to_string(),
                "Path" => p.path = v.trim().to_string(),
                "IsRelative" => p.is_relative = v.trim() == "1",
                _ => {}
            }
        }
    }
    if let Some(p) = cur {
        if !p.path.is_empty() {
            out.push(p);
        }
    }
    out
}

/// Extracts the quoted tokens from one line of a Valve KeyValues (.vdf/.acf) file.
fn vdf_tokens(line: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut tok = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        tok.push(n);
                    }
                }
                '"' => break,
                _ => tok.push(c),
            }
        }
        toks.push(tok);
    }
    toks
}

/// All `"path" "..."` values in libraryfolders.vdf.
pub fn parse_library_folders(vdf: &str) -> Vec<String> {
    vdf.lines()
        .map(vdf_tokens)
        .filter(|t| t.len() == 2 && t[0].eq_ignore_ascii_case("path"))
        .map(|t| t[1].clone())
        .collect()
}

/// Reads appid, name and SizeOnDisk from an appmanifest_*.acf.
pub fn parse_app_manifest(acf: &str) -> Option<SteamGame> {
    let mut appid = None;
    let mut name = None;
    let mut size = 0;
    for t in acf.lines().map(vdf_tokens).filter(|t| t.len() == 2) {
        match t[0].to_ascii_lowercase().as_str() {
            "appid" => appid = t[1].parse().ok(),
            "name" if name.is_none() => name = Some(t[1].clone()),
            "sizeondisk" => size = t[1].parse().unwrap_or(0),
            _ => {}
        }
    }
    Some(SteamGame { appid: appid?, name: name?, size_on_disk: size })
}

/// Chromium "Local State": profile.info_cache.<dir>.name
pub fn parse_chromium_local_state(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(cache) = v.pointer("/profile/info_cache").and_then(|c| c.as_object()) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = cache
        .iter()
        .map(|(dir, info)| {
            let name = info.get("name").and_then(|n| n.as_str()).unwrap_or(dir).to_string();
            (dir.clone(), name)
        })
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Filesystem walking (read-only, metadata only)
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn attrs(md: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes()
}
#[cfg(not(windows))]
fn attrs(_md: &std::fs::Metadata) -> u32 {
    0
}

const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x40000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x400000;

/// Walks `root` without following junctions/symlinks, calling `on_file` for
/// each regular file. Never reads file contents, so OneDrive placeholders are
/// not downloaded.
pub fn walk(root: &Path, mut on_file: impl FnMut(&Path, u64)) -> FolderStats {
    let mut stats = FolderStats::default();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            stats.unreadable += 1;
            continue;
        };
        for entry in rd {
            let Ok(entry) = entry else {
                stats.unreadable += 1;
                continue;
            };
            // DirEntry::metadata does not follow links on Windows.
            let Ok(md) = entry.metadata() else {
                stats.unreadable += 1;
                continue;
            };
            let a = attrs(&md);
            // std reports symlinks *and* junctions (name-surrogate reparse
            // points) as symlinks. OneDrive folders are also reparse points
            // but with a cloud tag, so they are (correctly) still walked.
            if md.file_type().is_symlink() {
                stats.links_skipped += 1;
                continue;
            }
            if md.is_dir() {
                stack.push(entry.path());
            } else {
                let len = md.len();
                stats.files += 1;
                stats.bytes += len;
                if a & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0 {
                    stats.cloud_only_files += 1;
                    stats.cloud_only_bytes += len;
                }
                on_file(&entry.path(), len);
            }
        }
    }
    stats
}

fn dir_bytes(p: &Path) -> u64 {
    walk(p, |_, _| {}).bytes
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

/// (label, User Shell Folders value name, default sub-folder of the profile)
const LIBRARY_FOLDERS: [(&str, &str, &str); 6] = [
    ("Documents", "Personal", "Documents"),
    ("Desktop", "Desktop", "Desktop"),
    ("Pictures", "My Pictures", "Pictures"),
    ("Music", "My Music", "Music"),
    ("Videos", "My Video", "Videos"),
    ("Downloads", "{374DE290-123F-4565-9164-39C4925E467B}", "Downloads"),
];

#[cfg(windows)]
fn shell_folder(value: &str) -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders")
        .ok()?
        .get_value::<String, _>(value)
        .ok()
}
#[cfg(not(windows))]
fn shell_folder(_value: &str) -> Option<String> {
    None
}

#[cfg(windows)]
fn steam_path() -> Option<PathBuf> {
    use winreg::enums::*;
    use winreg::RegKey;
    let p: String = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam")
        .ok()?
        .get_value("SteamPath")
        .ok()?;
    Some(PathBuf::from(p.replace('/', "\\")))
}
#[cfg(not(windows))]
fn steam_path() -> Option<PathBuf> {
    None
}

fn scan_folders(data: &mut UserData) {
    let profile = env("USERPROFILE").unwrap_or_default();
    let mut seen: Vec<PathBuf> = Vec::new();
    for (label, value, default) in LIBRARY_FOLDERS {
        let raw = shell_folder(value).unwrap_or_else(|| format!("{profile}\\{default}"));
        let path = PathBuf::from(expand_env(&raw, env));
        let exists = path.is_dir();
        // Two library folders can point at the same place; count it once.
        let dup = seen.iter().any(|s| s == &path);
        let stats = if exists && !dup {
            walk(&path, |f, _| {
                let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                match ext.as_str() {
                    "kdbx" => data.keepass_files.push(f.display().to_string()),
                    "pst" => data.outlook_pst_files.push(f.display().to_string()),
                    _ => {}
                }
            })
        } else {
            FolderStats::default()
        };
        seen.push(path.clone());
        data.folders.push(LibraryFolder { label: label.into(), path: path.display().to_string(), exists, stats });
    }
}

fn scan_browsers(data: &mut UserData) {
    let local = env("LOCALAPPDATA").unwrap_or_default();
    let roaming = env("APPDATA").unwrap_or_default();

    let chromium = [
        ("Google Chrome", "Google\\Chrome\\User Data"),
        ("Microsoft Edge", "Microsoft\\Edge\\User Data"),
        ("Brave", "BraveSoftware\\Brave-Browser\\User Data"),
        ("Vivaldi", "Vivaldi\\User Data"),
        ("Chromium", "Chromium\\User Data"),
    ];
    for (app, sub) in chromium {
        let root = Path::new(&local).join(sub);
        let Ok(state) = std::fs::read_to_string(root.join("Local State")) else { continue };
        for (dir, name) in parse_chromium_local_state(&state) {
            let p = root.join(&dir);
            if p.is_dir() {
                data.browsers.push(BrowserProfile { app: app.into(), profile_name: name, bytes: dir_bytes(&p), path: p.display().to_string() });
            }
        }
    }

    let mozilla = [
        ("Firefox", "Mozilla\\Firefox", false),
        ("LibreWolf", "librewolf", false),
        ("Waterfox", "Waterfox", false),
        ("Thunderbird", "Thunderbird", true),
    ];
    for (app, sub, is_mail) in mozilla {
        let root = Path::new(&roaming).join(sub);
        let Ok(ini) = std::fs::read_to_string(root.join("profiles.ini")) else { continue };
        for prof in parse_profiles_ini(&ini) {
            let p = if prof.is_relative { root.join(prof.path.replace('/', "\\")) } else { PathBuf::from(&prof.path) };
            if !p.is_dir() {
                continue;
            }
            let entry = BrowserProfile { app: app.into(), profile_name: prof.name, bytes: dir_bytes(&p), path: p.display().to_string() };
            if is_mail { data.mail.push(entry) } else { data.browsers.push(entry) }
        }
    }
}

fn scan_steam(data: &mut UserData) {
    let Some(steam) = steam_path() else { return };
    let vdf = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")).unwrap_or_default();
    let mut libs = parse_library_folders(&vdf);
    if libs.is_empty() {
        libs.push(steam.display().to_string());
    }
    for lib in libs {
        let apps = Path::new(&lib).join("steamapps");
        let mut games = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&apps) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with("appmanifest_") && name.ends_with(".acf") {
                    if let Some(g) = std::fs::read_to_string(e.path()).ok().as_deref().and_then(parse_app_manifest) {
                        games.push(g);
                    }
                }
            }
        }
        games.sort_by_key(|a| a.name.to_lowercase());
        let total_bytes = games.iter().map(|g| g.size_on_disk).sum();
        data.steam.push(SteamLibrary { path: lib, games, total_bytes });
    }
}

/// (name, base env var, sub path, note shown to the user)
const APP_DATA: &[(&str, &str, &str, &str)] = &[
    ("SSH keys", "USERPROFILE", ".ssh", "Copy to ~/.ssh on Linux; private keys must be chmod 600."),
    ("Git config", "USERPROFILE", ".gitconfig", "Copies across as ~/.gitconfig (check any Windows-only paths in it)."),
    ("OneDrive folder", "USERPROFILE", "OneDrive", "No official OneDrive client for Linux. Make sure files are downloaded (not cloud-only) before copying, or use rclone/onedriver."),
    ("Dropbox folder", "USERPROFILE", "Dropbox", "Dropbox has an official Linux client; it can re-sync instead of copying."),
    ("Saved Games", "USERPROFILE", "Saved Games", "Game saves. Many games sync through Steam Cloud; copy the rest."),
    ("My Games (in Documents)", "USERPROFILE", "Documents\\My Games", "Game saves/settings. Proton keeps these inside each game's prefix; copy per game if needed."),
    ("Discord", "APPDATA", "discord", "Nothing to copy: just log in again on Linux."),
    ("Spotify", "APPDATA", "Spotify", "Nothing to copy: log in again (offline downloads must be re-downloaded)."),
    ("OBS Studio settings", "APPDATA", "obs-studio", "Scenes and profiles can be imported in OBS on Linux (Scene Collection → Import). File paths inside need fixing."),
    ("VS Code settings", "APPDATA", "Code\\User", "settings.json, keybindings and snippets copy to ~/.config/Code/User, or use Settings Sync."),
    ("Minecraft (.minecraft)", "APPDATA", ".minecraft", "Worlds (saves/), resource packs and screenshots copy straight across."),
    ("Prism Launcher", "APPDATA", "PrismLauncher", "Instances copy to ~/.local/share/PrismLauncher (Flatpak: ~/.var/app/org.prismlauncher.PrismLauncher)."),
    ("Signal", "APPDATA", "Signal", "Desktop message history does not transfer; link the new Linux app to your phone."),
    ("Telegram", "APPDATA", "Telegram Desktop", "Cloud-based: log in again."),
    ("Slack", "APPDATA", "Slack", "Nothing to copy: log in again."),
    ("Microsoft Teams", "APPDATA", "Microsoft\\Teams", "Use Teams in the browser on Linux; nothing to copy."),
    ("KeePass settings", "APPDATA", "KeePass", "Only settings; your .kdbx database file is what matters (listed separately)."),
    ("KeePassXC settings", "LOCALAPPDATA", "KeePassXC", "Only settings; your .kdbx database file is what matters (listed separately)."),
    ("User-installed fonts", "LOCALAPPDATA", "Microsoft\\Windows\\Fonts", "Copy to ~/.local/share/fonts."),
];

fn scan_app_data(data: &mut UserData) {
    for (name, base, sub, note) in APP_DATA {
        let Some(b) = env(base) else { continue };
        let p = Path::new(&b).join(sub);
        let bytes = if p.is_dir() {
            dir_bytes(&p)
        } else if p.is_file() {
            std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
        } else {
            continue;
        };
        data.app_data.push(AppDataItem { name: (*name).into(), path: p.display().to_string(), bytes, note: (*note).into() });
    }
}

pub fn scan() -> UserData {
    let mut data = UserData::default();
    scan_folders(&mut data);
    scan_browsers(&mut data);
    scan_steam(&mut data);
    scan_app_data(&mut data);
    data.keepass_files.sort();
    data.keepass_files.dedup();
    data.outlook_pst_files.sort();
    data.outlook_pst_files.dedup();
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_expansion() {
        let look = |n: &str| match n {
            "USERPROFILE" => Some("C:\\Users\\Bob".to_string()),
            _ => None,
        };
        assert_eq!(expand_env("%USERPROFILE%\\Documents", look), "C:\\Users\\Bob\\Documents");
        assert_eq!(expand_env("%NOPE%\\x", look), "%NOPE%\\x");
        assert_eq!(expand_env("100% done", look), "100% done");
        assert_eq!(expand_env("D:\\Data", look), "D:\\Data");
    }

    #[test]
    fn profiles_ini() {
        let ini = "[Install4F96D1932A9F858E]\nDefault=Profiles/abc.default-release\n\n\
                   [Profile1]\nName=default\nIsRelative=1\nPath=Profiles/xyz.default\n\n\
                   [Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/abc.default-release\nDefault=1\n\n\
                   [Profile2]\nName=elsewhere\nIsRelative=0\nPath=D:\\FF\\p\n\n[General]\nStartWithLastProfile=1\n";
        let p = parse_profiles_ini(ini);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], IniProfile { name: "default".into(), path: "Profiles/xyz.default".into(), is_relative: true });
        assert_eq!(p[2].path, "D:\\FF\\p");
        assert!(!p[2].is_relative);
    }

    #[test]
    fn steam_vdf() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t\t\"apps\"\n\t\t{\n\t\t\t\"228980\"\t\t\"0\"\n\t\t}\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}\n";
        assert_eq!(parse_library_folders(vdf), vec!["C:\\Program Files (x86)\\Steam", "D:\\SteamLibrary"]);
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"620\"\n\t\"name\"\t\t\"Portal 2\"\n\t\"SizeOnDisk\"\t\t\"12884901888\"\n}\n";
        assert_eq!(parse_app_manifest(acf), Some(SteamGame { appid: 620, name: "Portal 2".into(), size_on_disk: 12884901888 }));
        assert_eq!(parse_app_manifest("garbage"), None);
    }

    #[test]
    fn chromium_local_state() {
        let j = r#"{"profile":{"info_cache":{"Profile 1":{"name":"Work"},"Default":{"name":"Person 1"}}}}"#;
        assert_eq!(parse_chromium_local_state(j), vec![("Default".into(), "Person 1".into()), ("Profile 1".into(), "Work".into())]);
        assert!(parse_chromium_local_state("not json").is_empty());
    }

    #[test]
    fn walk_counts_files_and_finds_extensions() {
        let base = std::env::temp_dir().join(format!("linuxify-walk-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("a/b")).unwrap();
        std::fs::write(base.join("a/one.txt"), b"12345").unwrap();
        std::fs::write(base.join("a/b/pw.kdbx"), b"123").unwrap();
        let mut found = Vec::new();
        let s = walk(&base, |p, _| {
            if p.extension().is_some_and(|e| e == "kdbx") {
                found.push(p.to_path_buf());
            }
        });
        std::fs::remove_dir_all(&base).unwrap();
        assert_eq!(s.files, 2);
        assert_eq!(s.bytes, 8);
        assert_eq!(found.len(), 1);
    }
}

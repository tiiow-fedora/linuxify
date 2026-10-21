//! Stage 4: copy (never move) the user's data into a Linux-friendly staging
//! folder, with a SHA-256 manifest, resume support and verification.
//!
//! Safety: sources are only ever *read*. Everything is written inside
//! `<target>\linuxify-migration\`, which must not overlap any source.

use crate::download::human;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

pub const STAGING_DIR: &str = "linuxify-migration";
pub const PROGRESS_FILE: &str = ".linuxify-progress.jsonl";
const PARTIAL_SUFFIX: &str = ".linuxify-partial";

/// What to copy and where it lands (relative to the staging root, `/`-separated).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceItem {
    pub label: String,
    pub from: PathBuf,
    pub to: String,
    /// Extra file/folder names to leave out (case-insensitive), e.g. browser caches.
    pub exclude: Vec<String>,
    /// Only these top-level children are copied (empty = everything).
    pub only: Vec<String>,
}

impl SourceItem {
    fn new(label: &str, from: PathBuf, to: &str) -> SourceItem {
        SourceItem { label: label.into(), from, to: to.into(), exclude: vec![], only: vec![] }
    }
}

/// One file to copy.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedFile {
    pub from: PathBuf,
    pub rel: String,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skipped {
    pub from: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub files: Vec<PlannedFile>,
    pub skipped: Vec<Skipped>,
    /// Bytes per source label, for the summary.
    pub per_source: BTreeMap<String, (u64, u64)>,
}

impl Plan {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// A finished file, as recorded in the progress log and the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopiedFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    /// Source modification time (Unix seconds); also set on the copy.
    pub mtime: i64,
    pub from: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub linuxify_version: String,
    pub created_at: String,
    pub computer: Option<String>,
    pub total_files: u64,
    pub total_bytes: u64,
    pub files: Vec<CopiedFile>,
    pub skipped: Vec<Skipped>,
}

// ---------------------------------------------------------------------------
// Pure rules
// ---------------------------------------------------------------------------

/// Windows clutter that has no meaning on Linux.
pub fn junk_reason(name: &str) -> Option<&'static str> {
    let l = name.to_ascii_lowercase();
    if matches!(l.as_str(), "desktop.ini" | "thumbs.db" | "ehthumbs.db" | "$recycle.bin" | "system volume information") {
        return Some("Windows system file");
    }
    if l.starts_with("~$") {
        return Some("Office temporary lock file");
    }
    if l.ends_with(".lnk") {
        return Some("Windows shortcut (doesn't work on Linux)");
    }
    if l.ends_with(PARTIAL_SUFFIX) {
        return Some("unfinished Linuxify copy");
    }
    None
}

/// Linux limits a single name to 255 *bytes* (UTF-8). Windows allows 255
/// UTF-16 units, so long non-ASCII names can be too long for Linux.
pub fn linux_name_problem(name: &str) -> Option<String> {
    if name.len() > 255 {
        return Some(format!("name is {} bytes; Linux allows 255", name.len()));
    }
    None
}

pub fn join_rel(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}/{name}")
    }
}

/// True if `inner` is `outer` or inside it (case-insensitive, Windows-style).
pub fn path_within(inner: &Path, outer: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
        let s = s.trim_start_matches(r"\\?\").trim_end_matches('\\').to_string();
        s
    };
    let (i, o) = (norm(inner), norm(outer));
    i == o || i.starts_with(&(o + "\\"))
}

/// Refuses targets that would overlap the data being copied or live in a
/// system folder.
pub fn check_target(staging: &Path, sources: &[SourceItem]) -> Result<(), String> {
    for s in sources {
        if path_within(staging, &s.from) {
            return Err(format!("the target is inside {} ({}), which is being copied; pick a folder outside it", s.from.display(), s.label));
        }
        if path_within(&s.from, staging) {
            return Err(format!("{} is inside the target folder; pick a different target", s.from.display()));
        }
    }
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    for sys in [windir.as_str(), r"C:\Program Files", r"C:\Program Files (x86)", r"C:\ProgramData"] {
        if path_within(staging, Path::new(sys)) {
            return Err(format!("refusing to write into a system folder ({sys})"));
        }
    }
    Ok(())
}

/// Drops sources nested inside other sources so nothing is copied twice.
pub fn dedupe_sources(mut items: Vec<SourceItem>) -> Vec<SourceItem> {
    let all: Vec<PathBuf> = items.iter().map(|i| i.from.clone()).collect();
    items.retain(|i| !all.iter().any(|o| o != &i.from && path_within(&i.from, o)));
    items
}

/// Reads the progress log. Later lines win; malformed lines (e.g. a line cut
/// short by a crash) are ignored.
pub fn parse_progress(text: &str) -> HashMap<String, CopiedFile> {
    let mut m = HashMap::new();
    for line in text.lines() {
        if let Ok(c) = serde_json::from_str::<CopiedFile>(line) {
            m.insert(c.path.clone(), c);
        }
    }
    m
}

/// `sha256sum -c` format, run from the staging root on Linux.
pub fn sha256sums(files: &[CopiedFile]) -> String {
    let mut out = String::new();
    for f in files {
        // GNU sha256sum escapes names containing backslash or newline with a leading '\'.
        if f.path.contains('\\') || f.path.contains('\n') {
            out.push_str(&format!("\\{}  ./{}\n", f.sha256, f.path.replace('\\', "\\\\").replace('\n', "\\n")));
        } else {
            out.push_str(&format!("{}  ./{}\n", f.sha256, f.path));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// What to migrate (default set, built from the user's real folders)
// ---------------------------------------------------------------------------

fn env(n: &str) -> Option<PathBuf> {
    std::env::var_os(n).filter(|v| !v.is_empty()).map(PathBuf::from)
}

const FIREFOX_EXCLUDE: &[&str] = &[
    "cache2", "startupcache", "thumbnails", "crashes", "minidumps", "datareporting", "saved-telemetry-pings",
    "lock", "parent.lock", ".parentlock", "shader-cache", "safebrowsing", "storage.sqlite-journal",
];

/// The default migration set, from the scan's folder list and known app data.
pub fn default_sources(data: &crate::scan::userdata::UserData) -> Vec<SourceItem> {
    let mut v = Vec::new();
    for f in data.folders.iter().filter(|f| f.exists) {
        v.push(SourceItem::new(&f.label, PathBuf::from(&f.path), &format!("home/{}", f.label)));
    }
    if let Some(p) = env("USERPROFILE") {
        let ssh = p.join(".ssh");
        if ssh.is_dir() {
            v.push(SourceItem::new("SSH keys", ssh, "home/.ssh"));
        }
        let gc = p.join(".gitconfig");
        if gc.is_file() {
            v.push(SourceItem::new("Git config", gc, "home/.gitconfig"));
        }
        let sg = p.join("Saved Games");
        if sg.is_dir() {
            v.push(SourceItem::new("Saved Games", sg, "home/Windows Saved Games"));
        }
    }
    if let Some(la) = env("LOCALAPPDATA") {
        let fonts = la.join(r"Microsoft\Windows\Fonts");
        if fonts.is_dir() {
            v.push(SourceItem::new("Fonts", fonts, "home/.local/share/fonts"));
        }
    }
    if let Some(ad) = env("APPDATA") {
        let code = ad.join(r"Code\User");
        if code.is_dir() {
            let mut s = SourceItem::new("VS Code settings", code, "home/.config/Code/User");
            s.only = vec!["settings.json".into(), "keybindings.json".into(), "snippets".into(), "tasks.json".into()];
            v.push(s);
        }
        let mc = ad.join(".minecraft");
        if mc.is_dir() {
            let mut s = SourceItem::new("Minecraft worlds", mc, "home/.minecraft");
            s.only = vec!["saves".into(), "resourcepacks".into(), "shaderpacks".into(), "screenshots".into(), "options.txt".into()];
            v.push(s);
        }
        let prism = ad.join(r"PrismLauncher\instances");
        if prism.is_dir() {
            v.push(SourceItem::new("Prism Launcher instances", prism, "home/.local/share/PrismLauncher/instances"));
        }
        let obs = ad.join(r"obs-studio\basic");
        if obs.is_dir() {
            v.push(SourceItem::new("OBS scenes and profiles", obs, "apps/obs-studio/basic"));
        }
    }
    for b in data.browsers.iter().chain(data.mail.iter()) {
        let safe: String = b.profile_name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        match b.app.as_str() {
            "Firefox" | "LibreWolf" | "Waterfox" | "Thunderbird" => {
                let kind = if b.app == "Thunderbird" { "mail" } else { "browsers" };
                let mut s = SourceItem::new(&format!("{} profile '{}'", b.app, b.profile_name), PathBuf::from(&b.path), &format!("{kind}/{}/{safe}", b.app.to_lowercase()));
                s.exclude = FIREFOX_EXCLUDE.iter().map(|x| x.to_string()).collect();
                v.push(s);
            }
            // Chromium browsers: passwords/cookies are encrypted with Windows
            // DPAPI and can't be read on Linux, so only bookmarks are staged.
            _ => {
                let mut s = SourceItem::new(&format!("{} bookmarks '{}'", b.app, b.profile_name), PathBuf::from(&b.path), &format!("browsers/{}/{safe}", b.app.to_lowercase().replace(' ', "-")));
                s.only = vec!["Bookmarks".into()];
                v.push(s);
            }
        }
    }
    dedupe_sources(v)
}

// ---------------------------------------------------------------------------
// Planning (walks the sources; reads metadata only)
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn attrs(md: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes()
}
#[cfg(not(windows))]
fn attrs(_: &std::fs::Metadata) -> u32 {
    0
}
const CLOUD_ONLY: u32 = 0x1000 | 0x40000 | 0x400000; // OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS

fn unix_secs(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn plan(sources: &[SourceItem]) -> Plan {
    let mut p = Plan::default();
    for s in sources {
        let before_files = p.files.len();
        let md = match std::fs::symlink_metadata(&s.from) {
            Ok(m) => m,
            Err(e) => {
                p.skipped.push(Skipped { from: s.from.display().to_string(), reason: format!("can't read: {e}") });
                continue;
            }
        };
        if md.is_file() {
            p.files.push(PlannedFile { from: s.from.clone(), rel: s.to.clone(), size: md.len(), mtime: unix_secs(md.modified()) });
        } else {
            walk_into(&mut p, s, &s.from, &s.to, true);
        }
        let added = &p.files[before_files..];
        p.per_source.insert(s.label.clone(), (added.len() as u64, added.iter().map(|f| f.size).sum()));
    }
    p
}

fn walk_into(p: &mut Plan, s: &SourceItem, dir: &Path, rel: &str, top: bool) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            p.skipped.push(Skipped { from: dir.display().to_string(), reason: format!("can't open folder: {e}") });
            return;
        }
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let lname = name.to_lowercase();
        let path = e.path();
        if top && !s.only.is_empty() && !s.only.iter().any(|o| o.eq_ignore_ascii_case(&name)) {
            continue;
        }
        if s.exclude.iter().any(|x| x.eq_ignore_ascii_case(&lname)) {
            continue; // known cache: silently left out
        }
        if let Some(r) = junk_reason(&name) {
            p.skipped.push(Skipped { from: path.display().to_string(), reason: r.into() });
            continue;
        }
        if let Some(r) = linux_name_problem(&name) {
            p.skipped.push(Skipped { from: path.display().to_string(), reason: r });
            continue;
        }
        let Ok(md) = e.metadata() else {
            p.skipped.push(Skipped { from: path.display().to_string(), reason: "can't read file details".into() });
            continue;
        };
        if md.file_type().is_symlink() {
            p.skipped.push(Skipped { from: path.display().to_string(), reason: "link/junction (not followed)".into() });
            continue;
        }
        let child_rel = join_rel(rel, &name);
        if md.is_dir() {
            walk_into(p, s, &path, &child_rel, false);
        } else if attrs(&md) & CLOUD_ONLY != 0 {
            p.skipped.push(Skipped { from: path.display().to_string(), reason: "online-only (OneDrive): make it 'Always keep on this device' and re-run".into() });
        } else {
            p.files.push(PlannedFile { from: path, rel: child_rel, size: md.len(), mtime: unix_secs(md.modified()) });
        }
    }
}

// ---------------------------------------------------------------------------
// Copying
// ---------------------------------------------------------------------------

fn hash_and_copy(from: &Path, to: &Path) -> std::io::Result<(String, u64)> {
    let mut src = std::fs::File::open(from)?;
    let mut dst = std::fs::File::create(to)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n_total = 0u64;
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        dst.write_all(&buf[..n])?;
        n_total += n as u64;
    }
    dst.flush()?;
    // Preserve modified/accessed (and on Windows, created) times.
    let md = src.metadata()?;
    let mut times = std::fs::FileTimes::new();
    if let Ok(m) = md.modified() {
        times = times.set_modified(m);
    }
    if let Ok(a) = md.accessed() {
        times = times.set_accessed(a);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTimesExt;
        if let Ok(c) = md.created() {
            times = times.set_created(c);
        }
    }
    dst.set_times(times)?;
    Ok((format!("{:x}", h.finalize()), n_total))
}

fn is_sharing_violation(e: &std::io::Error) -> bool {
    // ERROR_SHARING_VIOLATION (32) / ERROR_LOCK_VIOLATION (33)
    matches!(e.raw_os_error(), Some(32) | Some(33))
}

pub struct CopyOutcome {
    pub copied: Vec<CopiedFile>,
    pub reused: u64,
    pub newly_copied: u64,
    pub failed: Vec<Skipped>,
}

/// Copies every planned file into `staging`, resuming from the progress log.
/// `stop_after` (testing aid) stops after copying that many new files, as if
/// interrupted.
pub fn copy_all(plan: &Plan, staging: &Path, stop_after: Option<u64>, mut progress: impl FnMut(u64, u64, &str)) -> Result<CopyOutcome, String> {
    std::fs::create_dir_all(staging).map_err(|e| format!("could not create {}: {e}", staging.display()))?;
    let log_path = staging.join(PROGRESS_FILE);
    let done = parse_progress(&std::fs::read_to_string(&log_path).unwrap_or_default());
    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).map_err(|e| e.to_string())?;

    let total = plan.total_bytes();
    let mut bytes_done = 0u64;
    let mut out = CopyOutcome { copied: vec![], reused: 0, newly_copied: 0, failed: vec![] };
    for f in &plan.files {
        let dest = staging.join(f.rel.replace('/', "\\"));
        // Resume: same source size + mtime, and the copy is still there with the right size.
        if let Some(prev) = done.get(&f.rel) {
            let same_src = prev.size == f.size && prev.mtime == f.mtime;
            let dest_ok = std::fs::metadata(&dest).is_ok_and(|m| m.len() == f.size);
            if same_src && dest_ok {
                out.copied.push(prev.clone());
                out.reused += 1;
                bytes_done += f.size;
                progress(bytes_done, total, &f.rel);
                continue;
            }
        }
        if stop_after.is_some_and(|n| out.newly_copied >= n) {
            return Err(format!("stopped early after {} files (test interruption)", out.newly_copied));
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        let tmp = PathBuf::from(format!("{}{PARTIAL_SUFFIX}", dest.display()));
        match hash_and_copy(&f.from, &tmp) {
            Ok((sha256, n)) => {
                if n != f.size {
                    let _ = std::fs::remove_file(&tmp);
                    out.failed.push(Skipped { from: f.from.display().to_string(), reason: "changed while being copied; close the app using it and re-run".into() });
                    continue;
                }
                std::fs::rename(&tmp, &dest).map_err(|e| format!("could not finish {}: {e}", dest.display()))?;
                let c = CopiedFile { path: f.rel.clone(), size: f.size, sha256, mtime: f.mtime, from: f.from.display().to_string() };
                writeln!(log, "{}", serde_json::to_string(&c).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                out.copied.push(c);
                out.newly_copied += 1;
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                let reason = if is_sharing_violation(&e) {
                    "in use by another program (close it and re-run to pick it up)".to_string()
                } else if e.raw_os_error() == Some(112) {
                    return Err(format!("the target disk is full while copying {}", f.from.display()));
                } else {
                    format!("could not copy: {e}")
                };
                out.failed.push(Skipped { from: f.from.display().to_string(), reason });
            }
        }
        bytes_done += f.size;
        progress(bytes_done, total, &f.rel);
    }
    Ok(out)
}

pub fn write_manifest(staging: &Path, copied: &[CopiedFile], skipped: &[Skipped]) -> Result<Manifest, String> {
    let mut files = copied.to_vec();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let m = Manifest {
        linuxify_version: env!("CARGO_PKG_VERSION").into(),
        created_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        computer: std::env::var("COMPUTERNAME").ok(),
        total_files: files.len() as u64,
        total_bytes: files.iter().map(|f| f.size).sum(),
        files,
        skipped: skipped.to_vec(),
    };
    std::fs::write(staging.join("manifest.json"), serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::write(staging.join("SHA256SUMS"), sha256sums(&m.files)).map_err(|e| e.to_string())?;
    let mut flagged = String::from("Files Linuxify did not copy, and why.\r\n\r\n");
    for s in skipped {
        flagged.push_str(&format!("{}\r\n    {}\r\n", s.from, s.reason));
    }
    std::fs::write(staging.join("flagged.txt"), flagged).map_err(|e| e.to_string())?;
    Ok(m)
}

#[derive(Debug, Default)]
pub struct VerifyReport {
    pub ok: u64,
    pub missing: Vec<String>,
    pub mismatched: Vec<String>,
    pub extra: Vec<String>,
}

/// Re-hashes every file in the staging folder against manifest.json.
pub fn verify(staging: &Path, mut progress: impl FnMut(u64, u64)) -> Result<VerifyReport, String> {
    let text = std::fs::read_to_string(staging.join("manifest.json")).map_err(|e| format!("no manifest.json in {}: {e}", staging.display()))?;
    let m: Manifest = serde_json::from_str(&text).map_err(|e| format!("manifest.json is damaged: {e}"))?;
    let mut r = VerifyReport::default();
    let total = m.total_bytes.max(1);
    let mut done = 0u64;
    for f in &m.files {
        let p = staging.join(f.path.replace('/', "\\"));
        match crate::download::sha256_file(&p, |_| {}) {
            Ok(h) if h == f.sha256 => r.ok += 1,
            Ok(_) => r.mismatched.push(f.path.clone()),
            Err(_) => r.missing.push(f.path.clone()),
        }
        done += f.size;
        progress(done, total);
    }
    // Files in the staging area that the manifest doesn't know about.
    let known: std::collections::HashSet<&str> = m.files.iter().map(|f| f.path.as_str()).collect();
    for top in ["home", "browsers", "mail", "apps"] {
        let root = staging.join(top);
        if root.is_dir() {
            let _ = crate::scan::userdata::walk(&root, |p, _| {
                if let Ok(rel) = p.strip_prefix(staging) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    if !known.contains(rel.as_str()) && !rel.ends_with(PARTIAL_SUFFIX) {
                        r.extra.push(rel);
                    }
                }
            });
        }
    }
    Ok(r)
}

/// Lines of the progress log (for tests and diagnostics).
pub fn progress_entries(staging: &Path) -> usize {
    std::fs::File::open(staging.join(PROGRESS_FILE)).map(|f| std::io::BufReader::new(f).lines().count()).unwrap_or(0)
}

pub fn summary_line(label: &str, files: u64, bytes: u64) -> String {
    format!("  {label:<38} {files:>7} files  {:>10}", human(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("linuxify-mig-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn junk_and_name_rules() {
        assert!(junk_reason("desktop.ini").is_some());
        assert!(junk_reason("Thumbs.db").is_some());
        assert!(junk_reason("~$Report.docx").is_some());
        assert!(junk_reason("Steam.lnk").is_some());
        assert!(junk_reason("report.docx").is_none());
        assert!(linux_name_problem(&"a".repeat(255)).is_none());
        assert!(linux_name_problem(&"é".repeat(130)).is_some()); // 260 bytes in UTF-8
    }

    #[test]
    fn path_overlap() {
        assert!(path_within(Path::new(r"C:\Users\Bob\Documents\x"), Path::new(r"C:\Users\Bob\Documents")));
        assert!(path_within(Path::new(r"c:\users\bob\documents"), Path::new(r"C:\Users\Bob\Documents\")));
        assert!(!path_within(Path::new(r"C:\Users\Bob\Documents2"), Path::new(r"C:\Users\Bob\Documents")));
        let src = vec![SourceItem::new("Documents", PathBuf::from(r"C:\Users\Bob\Documents"), "home/Documents")];
        assert!(check_target(Path::new(r"C:\Users\Bob\Documents\backup\linuxify-migration"), &src).is_err());
        assert!(check_target(Path::new(r"D:\linuxify-migration"), &src).is_ok());
        assert!(check_target(Path::new(r"C:\Windows\Temp\linuxify-migration"), &src).is_err());
        let parent = vec![SourceItem::new("x", PathBuf::from(r"D:\linuxify-migration\home"), "home/x")];
        assert!(check_target(Path::new(r"D:\linuxify-migration"), &parent).is_err());
    }

    #[test]
    fn nested_sources_are_deduped() {
        let v = dedupe_sources(vec![
            SourceItem::new("Documents", PathBuf::from(r"C:\U\Documents"), "home/Documents"),
            SourceItem::new("My Games", PathBuf::from(r"C:\U\Documents\My Games"), "home/x"),
            SourceItem::new("Desktop", PathBuf::from(r"C:\U\Desktop"), "home/Desktop"),
        ]);
        assert_eq!(v.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Documents", "Desktop"]);
    }

    #[test]
    fn progress_log_tolerates_truncated_lines() {
        let good = CopiedFile { path: "home/a.txt".into(), size: 3, sha256: "aa".into(), mtime: 5, from: "C:\\a.txt".into() };
        let text = format!("{}\n{{\"path\":\"home/b.t", serde_json::to_string(&good).unwrap());
        let m = parse_progress(&text);
        assert_eq!(m.len(), 1);
        assert_eq!(m["home/a.txt"], good);
    }

    #[test]
    fn sha256sums_format() {
        let f = |p: &str| CopiedFile { path: p.into(), size: 1, sha256: "ab".into(), mtime: 0, from: String::new() };
        assert_eq!(sha256sums(&[f("home/My File.txt")]), "ab  ./home/My File.txt\n");
        assert_eq!(sha256sums(&[f("home/a\\b")]), "\\ab  ./home/a\\\\b\n");
    }

    fn fixture(root: &Path) {
        std::fs::create_dir_all(root.join("Docs/Sub dir/Ünïcode ✓")).unwrap();
        std::fs::write(root.join("Docs/a.txt"), b"hello").unwrap();
        std::fs::write(root.join("Docs/Sub dir/b.bin"), vec![7u8; 300_000]).unwrap();
        std::fs::write(root.join("Docs/Sub dir/Ünïcode ✓/c.md"), b"# hi").unwrap();
        std::fs::write(root.join("Docs/desktop.ini"), b"[x]").unwrap();
        std::fs::write(root.join("Docs/~$lock.docx"), b"x").unwrap();
    }

    #[test]
    fn plan_copy_resume_verify_roundtrip() {
        let base = tmpdir("rt");
        let src = base.join("src");
        fixture(&src);
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
        std::fs::File::options().write(true).open(src.join("Docs/a.txt")).unwrap().set_modified(old).unwrap();
        let sources = vec![SourceItem::new("Docs", src.join("Docs"), "home/Documents")];
        let staging = base.join("target").join(STAGING_DIR);
        check_target(&staging, &sources).unwrap();

        let p = plan(&sources);
        assert_eq!(p.files.len(), 3);
        assert_eq!(p.skipped.len(), 2); // desktop.ini + ~$ lock file
        assert!(p.files.iter().any(|f| f.rel == "home/Documents/Sub dir/Ünïcode ✓/c.md"));

        // Interrupted after one file...
        let e = copy_all(&p, &staging, Some(1), |_, _, _| {}).err().unwrap();
        assert!(e.contains("stopped early"));
        assert_eq!(progress_entries(&staging), 1);
        // ...then resumed: the first file is reused, the other two copied.
        let out = copy_all(&p, &staging, None, |_, _, _| {}).unwrap();
        assert_eq!(out.reused, 1);
        assert_eq!(out.newly_copied, 2);
        assert!(out.failed.is_empty());

        write_manifest(&staging, &out.copied, &p.skipped).unwrap();
        let r = verify(&staging, |_, _| {}).unwrap();
        assert_eq!(r.ok, 3);
        assert!(r.missing.is_empty() && r.mismatched.is_empty() && r.extra.is_empty(), "{r:?}");

        // Timestamps preserved.
        let copied_mtime = std::fs::metadata(staging.join("home/Documents/a.txt")).unwrap().modified().unwrap();
        assert_eq!(copied_mtime, old);

        // Tampering is caught.
        std::fs::write(staging.join("home/Documents/a.txt"), b"HELLO").unwrap();
        std::fs::write(staging.join("home/Documents/new.txt"), b"x").unwrap();
        let r = verify(&staging, |_, _| {}).unwrap();
        assert_eq!(r.mismatched, ["home/Documents/a.txt"]);
        assert_eq!(r.extra, ["home/Documents/new.txt"]);

        // Sources were never modified.
        assert_eq!(std::fs::read(src.join("Docs/a.txt")).unwrap(), b"hello");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn changed_source_is_recopied_on_resume() {
        let base = tmpdir("chg");
        let src = base.join("src");
        fixture(&src);
        let sources = vec![SourceItem::new("Docs", src.join("Docs"), "home/Documents")];
        let staging = base.join("t").join(STAGING_DIR);
        let out = copy_all(&plan(&sources), &staging, None, |_, _, _| {}).unwrap();
        assert_eq!(out.newly_copied, 3);
        std::fs::write(src.join("Docs/a.txt"), b"hello, changed").unwrap();
        let out2 = copy_all(&plan(&sources), &staging, None, |_, _, _| {}).unwrap();
        assert_eq!((out2.reused, out2.newly_copied), (2, 1));
        assert_eq!(std::fs::read(staging.join("home/Documents/a.txt")).unwrap(), b"hello, changed");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn only_filter_limits_top_level() {
        let base = tmpdir("only");
        std::fs::create_dir_all(base.join("mc/saves/World1")).unwrap();
        std::fs::create_dir_all(base.join("mc/versions/1.21")).unwrap();
        std::fs::write(base.join("mc/saves/World1/level.dat"), b"x").unwrap();
        std::fs::write(base.join("mc/versions/1.21/big.jar"), b"x").unwrap();
        std::fs::write(base.join("mc/options.txt"), b"x").unwrap();
        let mut s = SourceItem::new("mc", base.join("mc"), "home/.minecraft");
        s.only = vec!["saves".into(), "options.txt".into()];
        let rels: Vec<String> = plan(&[s]).files.into_iter().map(|f| f.rel).collect();
        assert_eq!(rels, ["home/.minecraft/options.txt", "home/.minecraft/saves/World1/level.dat"]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn locked_files_are_flagged_and_picked_up_later() {
        use std::os::windows::fs::OpenOptionsExt;
        let base = tmpdir("lock");
        let src = base.join("src");
        fixture(&src);
        let sources = vec![SourceItem::new("Docs", src.join("Docs"), "home/Documents")];
        let staging = base.join("t").join(STAGING_DIR);
        // Hold a.txt open with no sharing, like a program with an exclusive lock.
        let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(src.join("Docs/a.txt")).unwrap();
        let out = copy_all(&plan(&sources), &staging, None, |_, _, _| {}).unwrap();
        assert_eq!(out.newly_copied, 2);
        assert_eq!(out.failed.len(), 1);
        assert!(out.failed[0].reason.contains("in use"), "{:?}", out.failed);
        assert!(!staging.join("home/Documents/a.txt.linuxify-partial").exists());
        drop(lock);
        let out2 = copy_all(&plan(&sources), &staging, None, |_, _, _| {}).unwrap();
        assert_eq!((out2.reused, out2.newly_copied, out2.failed.len()), (2, 1, 0));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn long_paths_are_copied() {
        let base = tmpdir("long");
        // > 260 characters in total.
        let mut deep = base.join("src");
        for i in 0..12 {
            deep = deep.join(format!("a-fairly-long-folder-name-{i:02}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep.txt"), b"deep").unwrap();
        assert!(deep.join("deep.txt").as_os_str().len() > 260);
        let sources = vec![SourceItem::new("src", base.join("src"), "home/Deep")];
        let staging = base.join("t").join(STAGING_DIR);
        let p = plan(&sources);
        let out = copy_all(&p, &staging, None, |_, _, _| {}).unwrap();
        assert_eq!(out.newly_copied, 1, "{:?}", out.failed);
        std::fs::remove_dir_all(&base).unwrap();
    }
}

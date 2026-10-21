//! Generates the Linux side of the migration: restore.sh (run once on the
//! new system), apps.txt (editable list of apps to install) and README.txt.

use crate::mapping::{Group, MappedProgram, Status};
use std::collections::BTreeSet;

/// One line of apps.txt.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AppLine {
    /// `flatpak <app-id>  # <for which Windows program>`
    Flatpak { id: String, why: String },
    /// `pkg <apt-name> <dnf-name>  # ...` (- when unknown)
    Pkg { apt: String, dnf: String, why: String },
}

impl AppLine {
    pub fn render(&self) -> String {
        match self {
            AppLine::Flatpak { id, why } => format!("flatpak {id}  # {why}"),
            AppLine::Pkg { apt, dnf, why } => format!("pkg {apt} {dnf}  # {why}"),
        }
    }
}

/// Picks one install per installed Windows program that maps to something
/// installable: the native app's Flatpak, else the first alternative with a
/// Flatpak, else native apt/dnf package names. Blocked/unknown apps are left out.
pub fn app_lines(programs: &[MappedProgram]) -> Vec<AppLine> {
    let mut seen_ids = BTreeSet::new();
    let mut out = Vec::new();
    for m in programs.iter().filter(|m| m.group == Group::Mapped) {
        let Some(e) = &m.entry else { continue };
        let why = format!("replaces {}", e.name).replace('#', "");
        let line = match m.status {
            Status::Native | Status::WineOk => {
                let l = e.linux.as_ref();
                if let Some(id) = l.and_then(|l| l.flatpak.clone()) {
                    Some(AppLine::Flatpak { id, why: if m.status == Status::Native { format!("{} (Linux version)", e.name) } else { why } })
                } else {
                    l.filter(|l| l.apt.is_some() || l.dnf.is_some()).map(|l| AppLine::Pkg {
                        apt: l.apt.clone().unwrap_or("-".into()),
                        dnf: l.dnf.clone().unwrap_or("-".into()),
                        why: format!("{} (Linux version)", e.name),
                    })
                }
            }
            Status::Alternative => e.alternatives.iter().find_map(|a| a.flatpak.clone()).map(|id| AppLine::Flatpak { id, why }),
            Status::Blocked | Status::Unknown => None,
        };
        if let Some(l) = line {
            let key = match &l {
                AppLine::Flatpak { id, .. } => id.clone(),
                AppLine::Pkg { apt, dnf, .. } => format!("{apt}/{dnf}"),
            };
            if seen_ids.insert(key) {
                out.push(l);
            }
        }
    }
    out.sort();
    out
}

pub fn apps_txt(lines: &[AppLine]) -> String {
    let mut s = String::from(
        "# Apps restore.sh will offer to install. Edit freely: delete a line to skip it,\n\
         # or add your own. Formats:\n\
         #   flatpak <app-id>          (from Flathub; works on any distro)\n\
         #   pkg <apt-name> <dnf-name> (native package; use - if there's none for that family)\n\n",
    );
    for l in lines {
        s.push_str(&l.render());
        s.push('\n');
    }
    s
}

pub const RESTORE_SH: &str = include_str!("restore.sh");
pub const REMOVE_WINDOWS_SH: &str = include_str!("remove-windows.sh");

pub fn readme_txt(distro: Option<&str>, total_files: u64, total_human: &str) -> String {
    format!(
        "LINUXIFY MIGRATION FOLDER\r\n\
         =========================\r\n\r\n\
         This folder holds a COPY of your files ({total_files} files, {total_human}). Nothing was moved or\r\n\
         deleted on Windows.\r\n\r\n\
         On your new Linux system{}:\r\n\
         \x20 1. Open this folder in the file manager.\r\n\
         \x20 2. Right-click in the folder > Open in Terminal.\r\n\
         \x20 3. Type:  bash restore.sh --dry-run     (shows what it would do)\r\n\
         \x20 4. Then:  bash restore.sh               (does it)\r\n\r\n\
         restore.sh checks every file against SHA256SUMS first, copies files into your\r\n\
         home folder WITHOUT overwriting anything already there, fixes SSH key\r\n\
         permissions, sets up your Firefox/Thunderbird profiles, and offers to install\r\n\
         the apps listed in apps.txt.\r\n\r\n\
         What's here:\r\n\
         \x20 home/         your files, laid out the way Linux expects (Documents, .ssh, ...)\r\n\
         \x20 browsers/     Firefox-family profiles; Chrome/Edge/Brave bookmarks\r\n\
         \x20 mail/         Thunderbird profiles\r\n\
         \x20 apps/         settings for some apps (e.g. OBS scenes)\r\n\
         \x20 manifest.json every file's size and SHA-256 (also used by Linuxify later)\r\n\
         \x20 SHA256SUMS    the same checksums, for 'sha256sum -c'\r\n\
         \x20 flagged.txt   things that were NOT copied, and why\r\n\r\n\
         Saved passwords in Chrome, Edge and Brave are locked to Windows and can't be\r\n\
         copied. Before switching, export them (Settings > Passwords > Export) or\r\n\
         sign in to the browser's sync.\r\n",
        distro.map(|d| format!(" ({d})")).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{AppMap, EMBEDDED_APP_MAP};
    use crate::scan::programs::Program;

    fn prog(name: &str) -> Program {
        Program { name: name.into(), publisher: None, version: None, sources: vec![], winget_id: None, steam_appid: None, microsoft_msix: false }
    }

    #[test]
    fn app_lines_from_real_map() {
        let map = AppMap::parse(EMBEDDED_APP_MAP).unwrap();
        let progs: Vec<_> = ["Discord", "Mozilla Firefox (x64 en-US)", "Microsoft Office Professional", "Fortnite", "Git", "Logitech G HUB", "Some Unknown Tool"]
            .iter()
            .map(|n| crate::mapping::classify(&prog(n), &map))
            .collect();
        let lines = app_lines(&progs);
        let r: Vec<String> = lines.iter().map(|l| l.render()).collect();
        assert!(r.iter().any(|l| l.starts_with("flatpak com.discordapp.Discord")), "{r:?}");
        assert!(r.iter().any(|l| l.starts_with("flatpak org.mozilla.firefox")), "{r:?}");
        assert!(r.iter().any(|l| l.starts_with("flatpak org.libreoffice.LibreOffice") && l.contains("replaces Microsoft Office")), "{r:?}");
        assert!(r.iter().any(|l| l.starts_with("pkg git git")), "{r:?}");
        assert!(!r.iter().any(|l| l.to_lowercase().contains("fortnite")));
        // Logitech G HUB's alternative (Solaar) has no Flatpak, so nothing is invented.
        assert!(!r.iter().any(|l| l.contains("G HUB")));
    }

    #[test]
    fn duplicates_collapse() {
        let map = AppMap::parse(EMBEDDED_APP_MAP).unwrap();
        let progs: Vec<_> = ["Microsoft Word", "Microsoft Excel"].iter().map(|n| crate::mapping::classify(&prog(n), &map)).collect();
        let lines = app_lines(&progs);
        assert_eq!(lines.iter().filter(|l| matches!(l, AppLine::Flatpak { id, .. } if id == "org.libreoffice.LibreOffice")).count(), 1);
    }

    #[test]
    fn restore_script_is_bash_and_safe() {
        assert!(RESTORE_SH.starts_with("#!/usr/bin/env bash"));
        assert!(!RESTORE_SH.contains("rm -rf"));
        assert!(RESTORE_SH.contains("sha256sum"));
        assert!(!RESTORE_SH.contains('\r'), "restore.sh must have Unix line endings");
    }
}

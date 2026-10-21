//! Official ISO sources (data/isos.json) and the pure logic around them:
//! parsing checksum files, picking the newest matching ISO, building URLs,
//! and enforcing that checksums only come from the distro's own servers.

use serde::{Deserialize, Serialize};

pub const EMBEDDED_ISOS: &str = include_str!("../data/isos.json");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IsoSource {
    pub distro: String,
    /// Official checksum file (SHA256SUMS-style, BSD-style, or single line).
    pub checksum_url: String,
    /// Hosts the checksum file may be served from (after redirects).
    pub allowed_hosts: Vec<String>,
    /// The ISO's file name must start with `prefix`, contain `contains`
    /// and end with `suffix`.
    pub prefix: String,
    #[serde(default)]
    pub contains: String,
    pub suffix: String,
    /// Optional official download folder (e.g. Fedora's mirror redirector)
    /// used instead of the checksum file's folder. Integrity still comes from
    /// the checksum fetched from `allowed_hosts`.
    #[serde(default)]
    pub download_base: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsoList {
    pub schema_version: u32,
    #[serde(default, rename = "_note")]
    pub note: String,
    pub sources: Vec<IsoSource>,
}

impl IsoList {
    pub fn parse(json: &str) -> Result<IsoList, String> {
        let l: IsoList = serde_json::from_str(json).map_err(|e| format!("isos.json is not valid: {e}"))?;
        for s in &l.sources {
            let host = host_of(&s.checksum_url).ok_or_else(|| format!("{}: checksum_url must be https", s.distro))?;
            if !s.allowed_hosts.iter().any(|h| h == host) {
                return Err(format!("{}: checksum_url host {host} is not in allowed_hosts", s.distro));
            }
            if let Some(b) = &s.download_base {
                if host_of(b).is_none() || !b.ends_with('/') {
                    return Err(format!("{}: download_base must be an https folder URL ending in /", s.distro));
                }
            }
        }
        Ok(l)
    }

    pub fn load(data_dir: Option<&std::path::Path>) -> Result<IsoList, String> {
        if let Some(d) = data_dir {
            if let Ok(t) = std::fs::read_to_string(d.join("isos.json")) {
                return IsoList::parse(&t);
            }
        }
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())) {
            if let Ok(t) = std::fs::read_to_string(exe_dir.join("data").join("isos.json")) {
                return IsoList::parse(&t);
            }
        }
        IsoList::parse(EMBEDDED_ISOS)
    }

    pub fn find(&self, distro: &str) -> Option<&IsoSource> {
        self.sources.iter().find(|s| s.distro == distro)
    }
}

/// Host of an https:// URL (None for anything that isn't https).
pub fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?; // never trust userinfo
    let host = host.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

/// Parses checksum files. Handles:
///   `<hash>  name`, `<hash> *name` (GNU), `SHA256 (name) = <hash>` (BSD),
///   PGP-signed wrappers, comments and blank lines.
/// Returns (file name, lowercase hex hash) pairs.
pub fn parse_checksums(text: &str) -> Vec<(String, String)> {
    let is_hash = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with("-----") || line.starts_with("Hash:") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("SHA256 (") {
            if let Some((name, hash)) = rest.split_once(") = ") {
                if is_hash(hash.trim()) {
                    out.push((name.to_string(), hash.trim().to_ascii_lowercase()));
                }
            }
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else { continue };
        let name = name.trim().trim_start_matches('*');
        if is_hash(hash) && !name.is_empty() && !name.contains('/') && !name.contains('\\') {
            out.push((name.to_string(), hash.to_ascii_lowercase()));
        }
    }
    out
}

/// Compares file names so that embedded numbers sort numerically:
/// "ubuntu-26.04.1" > "ubuntu-26.04", "44-1.10" > "44-1.7".
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut v: Vec<(bool, String)> = Vec::new();
        for c in s.chars() {
            let d = c.is_ascii_digit();
            match v.last_mut() {
                Some((ld, buf)) if *ld == d => buf.push(c),
                _ => v.push((d, c.to_string())),
            }
        }
        v
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for (x, y) in ca.iter().zip(cb.iter()) {
        let o = match (x.0, y.0) {
            (true, true) => {
                let (nx, ny) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
                nx.len().cmp(&ny.len()).then_with(|| nx.cmp(ny))
            }
            _ => {
                // "." sorts after the end of a name so "26.04.1" > "26.04-"
                x.1.cmp(&y.1)
            }
        };
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    ca.len().cmp(&cb.len())
}

/// The newest entry whose name matches the source's selector.
pub fn pick<'a>(src: &IsoSource, entries: &'a [(String, String)]) -> Option<&'a (String, String)> {
    entries
        .iter()
        .filter(|(n, _)| n.starts_with(&src.prefix) && n.contains(&src.contains) && n.ends_with(&src.suffix))
        .max_by(|a, b| natural_cmp(&a.0, &b.0))
}

/// ISOs live next to their checksum file: the folder of the *final*
/// (post-redirect) checksum URL + the file name.
pub fn iso_url(final_checksum_url: &str, file_name: &str) -> String {
    let dir = match final_checksum_url.rfind('/') {
        Some(i) => &final_checksum_url[..=i],
        None => final_checksum_url,
    };
    format!("{dir}{file_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const H1: &str = "487f87faaf547ea30e0aba4d5b53346292571256b25333a978db1692bcee9dd2";
    const H2: &str = "601e30fbf5d97759367c632e2c33630665039b7e2158fd068403da3ccf1bda1f";

    fn src(prefix: &str, contains: &str, suffix: &str) -> IsoSource {
        IsoSource { distro: "x".into(), checksum_url: "https://a.org/x".into(), allowed_hosts: vec!["a.org".into()], prefix: prefix.into(), contains: contains.into(), suffix: suffix.into(), download_base: None }
    }

    #[test]
    fn parses_gnu_star_and_bsd_formats() {
        let gnu = format!("{H1} *ubuntu-26.04-desktop-amd64.iso\n{H2} *ubuntu-26.04.1-desktop-amd64.iso\n");
        let e = parse_checksums(&gnu);
        assert_eq!(e, vec![("ubuntu-26.04-desktop-amd64.iso".into(), H1.into()), ("ubuntu-26.04.1-desktop-amd64.iso".into(), H2.into())]);
        let two_space = format!("{H1}  archlinux-2026.10.01-x86_64.iso\n");
        assert_eq!(parse_checksums(&two_space)[0].0, "archlinux-2026.10.01-x86_64.iso");
        let bsd = format!("-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\n# Fedora-Workstation-Live-44-1.7.x86_64.iso: 2500000000 bytes\nSHA256 (Fedora-Workstation-Live-44-1.7.x86_64.iso) = {}\n-----BEGIN PGP SIGNATURE-----\nabc\n", H1.to_uppercase());
        assert_eq!(parse_checksums(&bsd), vec![("Fedora-Workstation-Live-44-1.7.x86_64.iso".into(), H1.into())]);
    }

    #[test]
    fn rejects_paths_and_short_hashes() {
        let t = format!("abc  short.iso\n{H1}  ../evil.iso\n{H1}  dir/x.iso\n");
        assert!(parse_checksums(&t).is_empty());
    }

    #[test]
    fn natural_ordering() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("ubuntu-26.04.1-desktop", "ubuntu-26.04-desktop"), Greater);
        assert_eq!(natural_cmp("Fedora-44-1.10", "Fedora-44-1.7"), Greater);
        assert_eq!(natural_cmp("debian-live-13.10.0", "debian-live-13.7.0"), Greater);
        assert_eq!(natural_cmp("a", "a"), Equal);
    }

    #[test]
    fn picks_newest_matching_iso() {
        let e = parse_checksums(&format!("{H1} *ubuntu-26.04-desktop-amd64.iso\n{H2} *ubuntu-26.04.1-desktop-amd64.iso\n{H1} *ubuntu-26.04.1-live-server-amd64.iso\n"));
        let p = pick(&src("ubuntu-", "-desktop-", "amd64.iso"), &e).unwrap();
        assert_eq!(p.0, "ubuntu-26.04.1-desktop-amd64.iso");
        assert_eq!(p.1, H2);
        // Debian's list has .iso.contents/.log next to each .iso.
        let d = parse_checksums(&format!("{H1}  debian-live-13.7.0-amd64-gnome.iso\n{H2}  debian-live-13.7.0-amd64-gnome.iso.contents\n"));
        assert_eq!(pick(&src("debian-live-", "-amd64-gnome", ".iso"), &d).unwrap().1, H1);
        // Arch: the dated name, not the undated alias or the bootstrap tarball.
        let a = parse_checksums(&format!("{H1}  archlinux-2026.10.01-x86_64.iso\n{H1}  archlinux-x86_64.iso\n{H2}  archlinux-bootstrap-2026.10.01-x86_64.tar.zst\n"));
        assert_eq!(pick(&src("archlinux-20", "", "-x86_64.iso"), &a).unwrap().0, "archlinux-2026.10.01-x86_64.iso");
        assert!(pick(&src("nothing", "", ".iso"), &a).is_none());
    }

    #[test]
    fn hosts_and_urls() {
        assert_eq!(host_of("https://dl.fedoraproject.org/pub/x"), Some("dl.fedoraproject.org"));
        assert_eq!(host_of("https://evil.com@dl.fedoraproject.org/x"), Some("dl.fedoraproject.org"));
        assert_eq!(host_of("https://a.org:443/x"), Some("a.org"));
        assert_eq!(host_of("http://a.org/x"), None);
        assert_eq!(iso_url("https://download.opensuse.org/tumbleweed/iso/x-Snapshot1-Media.iso.sha256", "x-Snapshot1-Media.iso"), "https://download.opensuse.org/tumbleweed/iso/x-Snapshot1-Media.iso");
    }

    #[test]
    fn embedded_list_is_valid_and_matches_distros() {
        let l = IsoList::parse(EMBEDDED_ISOS).unwrap();
        let distros = crate::chooser::DistroList::parse(crate::chooser::EMBEDDED_DISTROS).unwrap();
        for s in &l.sources {
            assert!(distros.distros.iter().any(|d| d.id == s.distro), "unknown distro {}", s.distro);
        }
        assert!(l.sources.len() >= 10);
    }

    #[test]
    fn rejects_checksum_from_unlisted_host() {
        let bad = r#"{"schema_version":1,"sources":[{"distro":"x","checksum_url":"https://mirror.example.com/SUMS","allowed_hosts":["official.org"],"prefix":"x","suffix":".iso"}]}"#;
        assert!(IsoList::parse(bad).unwrap_err().contains("not in allowed_hosts"));
    }
}

//! Downloads with HTTPS-only, resume and SHA-256 verification.
//! Network and disk access live here; decisions (what to fetch, from where)
//! are made in iso.rs.

use crate::iso::{self, IsoSource};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const USER_AGENT: &str = concat!("Linuxify/", env!("CARGO_PKG_VERSION"), " (ISO downloader)");

fn client(timeout: Option<Duration>) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .https_only(true)
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("could not set up HTTPS: {e}"))
}

/// What we know about the ISO before downloading it.
#[derive(Debug, Clone)]
pub struct ResolvedIso {
    pub file_name: String,
    pub sha256: String,
    pub url: String,
    pub checksum_url: String,
}

/// Fetches the official checksum file (refusing it if any redirect lands
/// off the distro's own hosts) and picks the ISO to download.
pub fn resolve(src: &IsoSource) -> Result<ResolvedIso, String> {
    let resp = client(Some(Duration::from_secs(60)))?
        .get(&src.checksum_url)
        .send()
        .map_err(|e| format!("could not fetch the checksum list from {}: {e}", src.checksum_url))?;
    let final_url = resp.url().to_string();
    let host = iso::host_of(&final_url).unwrap_or("");
    if !src.allowed_hosts.iter().any(|h| h == host) {
        return Err(format!("the checksum list was redirected to {host}, which is not an official {} server; refusing", src.distro));
    }
    if !resp.status().is_success() {
        return Err(format!("{} returned HTTP {} (the release may have moved; data/isos.json needs updating)", final_url, resp.status()));
    }
    let text = resp.text().map_err(|e| e.to_string())?;
    let entries = iso::parse_checksums(&text);
    let (name, hash) = iso::pick(src, &entries)
        .ok_or_else(|| format!("no ISO matching '{}*{}*{}' in {final_url} (data/isos.json needs updating)", src.prefix, src.contains, src.suffix))?
        .clone();
    let url = match &src.download_base {
        Some(base) => format!("{base}{name}"),
        None => iso::iso_url(&final_url, &name),
    };
    Ok(ResolvedIso { url, file_name: name, sha256: hash, checksum_url: final_url })
}

pub fn sha256_file(path: &Path, mut progress: impl FnMut(u64)) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        done += n as u64;
        progress(done);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn marker_path(iso: &Path) -> PathBuf {
    let mut p = iso.as_os_str().to_owned();
    p.push(".verified");
    PathBuf::from(p)
}

/// True when a previous run already verified this exact file.
pub fn already_verified(iso: &Path, sha256: &str) -> bool {
    let (Ok(md), Ok(marker)) = (std::fs::metadata(iso), std::fs::read_to_string(marker_path(iso))) else {
        return false;
    };
    marker.trim() == format!("{sha256} {}", md.len())
}

fn write_marker(iso: &Path, sha256: &str) -> Result<(), String> {
    let len = std::fs::metadata(iso).map_err(|e| e.to_string())?.len();
    std::fs::write(marker_path(iso), format!("{sha256} {len}")).map_err(|e| e.to_string())
}

pub fn human(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.2} GB", bytes as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.0} MB", bytes as f64 / (1u64 << 20) as f64)
    }
}

fn verify_with_progress(path: &Path, expected: &str) -> Result<bool, String> {
    let total = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0).max(1);
    let mut last = Instant::now();
    print!("  Checking SHA-256...");
    let got = sha256_file(path, |done| {
        if last.elapsed() > Duration::from_secs(2) {
            print!("\r  Checking SHA-256... {:>3}%", done * 100 / total);
            let _ = std::io::stdout().flush();
            last = Instant::now();
        }
    })?;
    println!("\r  Checking SHA-256... done      ");
    Ok(got.eq_ignore_ascii_case(expected))
}

/// Makes sure `dir/<file_name>` exists and matches the official SHA-256,
/// downloading (with resume) if needed. Never returns an unverified path.
pub fn ensure_iso(r: &ResolvedIso, dir: &Path, reverify: bool) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let iso = dir.join(&r.file_name);
    if iso.exists() {
        if !reverify && already_verified(&iso, &r.sha256) {
            println!("  Already downloaded and verified: {}", iso.display());
            return Ok(iso);
        }
        if verify_with_progress(&iso, &r.sha256)? {
            write_marker(&iso, &r.sha256)?;
            println!("  Existing download verified.");
            return Ok(iso);
        }
        println!("  The existing file does not match the official checksum; downloading it again.");
        std::fs::remove_file(&iso).map_err(|e| e.to_string())?;
    }

    let part = dir.join(format!("{}.part", r.file_name));
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let c = client(None)?;
    let mut req = c.get(&r.url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut resp = req.send().map_err(|e| format!("download failed to start: {e}"))?;
    let status = resp.status();
    let resuming = have > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
    if !status.is_success() {
        return Err(format!("download failed: HTTP {status} from {}", resp.url()));
    }
    let remaining = resp.content_length().unwrap_or(0);
    let total = if resuming { have + remaining } else { remaining };
    if let Ok(free) = fs2::available_space(dir) {
        if remaining + 512 * 1024 * 1024 > free {
            return Err(format!("not enough free space in {}: need {} more, have {}", dir.display(), human(remaining), human(free)));
        }
    }
    println!(
        "  {} {} from {}",
        if resuming { "Resuming" } else { "Downloading" },
        human(total),
        iso::host_of(resp.url().as_str()).unwrap_or("?")
    );

    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resuming)
        .truncate(!resuming)
        .open(&part)
        .map_err(|e| format!("could not write {}: {e}", part.display()))?;
    let mut done = if resuming { have } else { 0 };
    let start = Instant::now();
    let start_done = done;
    let mut last = Instant::now();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = resp.read(&mut buf).map_err(|e| format!("download interrupted ({e}); run the same command again to resume"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if last.elapsed() > Duration::from_secs(2) {
            let rate = (done - start_done) as f64 / start.elapsed().as_secs_f64().max(0.1);
            let pct = if total > 0 { done * 100 / total } else { 0 };
            print!("\r  {pct:>3}%  {} / {}  {:.1} MB/s   ", human(done), human(total), rate / 1e6);
            let _ = std::io::stdout().flush();
            last = Instant::now();
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    drop(out);
    println!("\r  Downloaded {}.                              ", human(done));
    if total > 0 && done != total {
        return Err(format!("download ended early ({} of {}); run the same command again to resume", human(done), human(total)));
    }

    if !verify_with_progress(&part, &r.sha256)? {
        // Our own temporary file inside %LOCALAPPDATA%\Linuxify: safe to remove.
        let _ = std::fs::remove_file(&part);
        return Err("the download does NOT match the official SHA-256 checksum, so it was deleted and will not be used. Try again later or from another network.".into());
    }
    std::fs::rename(&part, &iso).map_err(|e| e.to_string())?;
    write_marker(&iso, &r.sha256)?;
    println!("  Verified against the official checksum.");
    Ok(iso)
}

/// Uses an ISO the user already has (e.g. on a NAS) only if it matches the
/// official checksum. The file is read, never moved or changed.
pub fn verify_local(path: &Path, r: &ResolvedIso) -> Result<PathBuf, String> {
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if verify_with_progress(path, &r.sha256)? {
        Ok(path.to_path_buf())
    } else {
        Err(format!("{} does not match the official checksum for {} ({}); not using it", path.display(), r.file_name, r.sha256))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_known_content() {
        let p = std::env::temp_dir().join(format!("linuxify-sha-{}", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        let h = sha256_file(&p, |_| {}).unwrap();
        std::fs::remove_file(&p).unwrap();
        assert_eq!(h, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn marker_requires_same_hash_and_size() {
        let p = std::env::temp_dir().join(format!("linuxify-marker-{}.iso", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        write_marker(&p, "aa").unwrap();
        assert!(already_verified(&p, "aa"));
        assert!(!already_verified(&p, "bb"));
        std::fs::write(&p, b"abcd").unwrap(); // size changed
        assert!(!already_verified(&p, "aa"));
        std::fs::remove_file(&p).unwrap();
        std::fs::remove_file(marker_path(&p)).unwrap();
    }
}

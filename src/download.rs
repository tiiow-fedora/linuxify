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
    let resumed = download_to_part(r, dir, &part)?;
    if !verify_with_progress(&part, &r.sha256)? {
        // Our own temporary files inside %LOCALAPPDATA%\Linuxify: safe to remove.
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(ok_path(&part));
        if !resumed {
            return Err("the download does NOT match the official SHA-256 checksum, so it was deleted and will not be used. Try again later or from another network.".into());
        }
        // A resumed download can carry damage from a crash; try once from scratch.
        println!("  The resumed download didn't match the checksum; downloading it again from the start.");
        download_to_part(r, dir, &part)?;
        if !verify_with_progress(&part, &r.sha256)? {
            let _ = std::fs::remove_file(&part);
            let _ = std::fs::remove_file(ok_path(&part));
            return Err("the download does NOT match the official SHA-256 checksum, so it was deleted and will not be used. Try again later or from another network.".into());
        }
    }
    std::fs::rename(&part, &iso).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(ok_path(&part));
    write_marker(&iso, &r.sha256)?;
    println!("  Verified against the official checksum.");
    Ok(iso)
}

/// How often the partial file is flushed to disk and its "known good"
/// length recorded, so a crash or power cut can't leave unwritten garbage
/// inside the part we resume from.
const CHECKPOINT_BYTES: u64 = 64 * 1024 * 1024;

fn ok_path(part: &Path) -> PathBuf {
    let mut p = part.as_os_str().to_owned();
    p.push(".ok");
    PathBuf::from(p)
}

/// Where to resume: never past the last checkpoint that was flushed to disk.
/// A .part with no checkpoint record is not trusted at all.
pub fn resume_point(part_len: u64, checkpoint: Option<u64>) -> u64 {
    checkpoint.map(|c| c.min(part_len)).unwrap_or(0)
}

/// Downloads into `part`, resuming from the last checkpoint. Returns true if
/// it resumed (rather than starting from zero).
fn download_to_part(r: &ResolvedIso, dir: &Path, part: &Path) -> Result<bool, String> {
    let part_len = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    let checkpoint = std::fs::read_to_string(ok_path(part)).ok().and_then(|t| t.trim().parse::<u64>().ok());
    let have = resume_point(part_len, checkpoint);
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
        if resuming { format!("Resuming at {} of", human(have)) } else { "Downloading".to_string() },
        human(total),
        iso::host_of(resp.url().as_str()).unwrap_or("?")
    );

    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(part)
        .map_err(|e| format!("could not write {}: {e}", part.display()))?;
    // Drop anything after the last checkpoint (it may be unflushed garbage).
    let start_at = if resuming { have } else { 0 };
    out.set_len(start_at).map_err(|e| e.to_string())?;
    std::io::Seek::seek(&mut out, std::io::SeekFrom::Start(start_at)).map_err(|e| e.to_string())?;
    std::fs::write(ok_path(part), start_at.to_string()).map_err(|e| e.to_string())?;

    let mut done = start_at;
    let mut since_checkpoint = 0u64;
    let start = Instant::now();
    let mut last = Instant::now();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = resp.read(&mut buf).map_err(|e| format!("download interrupted ({e}); run the same command again to resume"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        since_checkpoint += n as u64;
        if since_checkpoint >= CHECKPOINT_BYTES {
            out.sync_data().map_err(|e| e.to_string())?;
            std::fs::write(ok_path(part), done.to_string()).map_err(|e| e.to_string())?;
            since_checkpoint = 0;
        }
        if last.elapsed() > Duration::from_secs(2) {
            let rate = (done - start_at) as f64 / start.elapsed().as_secs_f64().max(0.1);
            let pct = if total > 0 { done * 100 / total } else { 0 };
            print!("\r  {pct:>3}%  {} / {}  {:.1} MB/s   ", human(done), human(total), rate / 1e6);
            let _ = std::io::stdout().flush();
            last = Instant::now();
        }
    }
    out.sync_data().map_err(|e| e.to_string())?;
    drop(out);
    std::fs::write(ok_path(part), done.to_string()).map_err(|e| e.to_string())?;
    println!("\r  Downloaded {}.                              ", human(done));
    if total > 0 && done != total {
        return Err(format!("download ended early ({} of {}); run the same command again to resume", human(done), human(total)));
    }
    Ok(resuming)
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
    fn resume_never_trusts_more_than_the_checkpoint() {
        assert_eq!(resume_point(2_104_492_032, None), 0, "no checkpoint record: start over");
        assert_eq!(resume_point(2_104_492_032, Some(2_013_265_920)), 2_013_265_920);
        assert_eq!(resume_point(1_000, Some(5_000)), 1_000, "file shorter than checkpoint");
        assert_eq!(resume_point(0, Some(0)), 0);
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

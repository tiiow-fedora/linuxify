//! Stage 5 executor: the only code in Linuxify that changes disks or boot
//! settings. Every function here re-checks the real state before acting,
//! and install-state.json is written before the first change.

use crate::install::{self, InstallPlan, InstallState, Phase};
use crate::scan::hardware::{self, HardwareFacts};
use std::path::{Path, PathBuf};

pub fn state_path() -> Result<PathBuf, String> {
    Ok(crate::appdirs::home()?.join("install-state.json"))
}

pub fn load_state() -> Option<InstallState> {
    serde_json::from_str(&std::fs::read_to_string(state_path().ok()?).ok()?).ok()
}

pub fn save_state(s: &InstallState) -> Result<(), String> {
    let p = state_path()?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&p, serde_json::to_string_pretty(s).unwrap()).map_err(|e| format!("could not save install state: {e}"))
}

fn ps(script: &str) -> Result<String, String> {
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// First free drive letter, preferring L (for Linux).
pub fn free_letter(hw: &HardwareFacts) -> Option<char> {
    let used: Vec<char> = hw.disks.iter().flat_map(|d| d.partitions.iter().filter_map(|p| p.drive_letter)).collect();
    "LMNOPQRSTUVWXYZHIJK".chars().find(|c| !used.contains(c) && !Path::new(&format!("{c}:\\")).exists())
}

/// Disk number holding a drive letter (None for network paths / unknown).
pub fn disk_of_letter(hw: &HardwareFacts, letter: char) -> Option<u32> {
    let l = letter.to_ascii_uppercase();
    hw.disks.iter().find(|d| d.partitions.iter().any(|p| p.drive_letter == Some(l))).map(|d| d.number)
}

/// Mounts the ISO read-only and returns (drive letter, file count, total bytes, largest file).
pub fn mount_iso(iso: &Path) -> Result<(char, u64, u64, u64), String> {
    let p = iso.display().to_string().replace('\'', "''");
    let out = ps(&format!(
        "$i = Mount-DiskImage -ImagePath '{p}' -Access ReadOnly -PassThru; Start-Sleep 1; \
         $l = ($i | Get-Volume).DriveLetter; \
         $f = Get-ChildItem \"$($l):\\\" -Recurse -File -Force; \
         \"$l|$($f.Count)|$(($f | Measure-Object Length -Sum).Sum)|$(($f | Measure-Object Length -Maximum).Maximum)\""
    ))?;
    let parts: Vec<&str> = out.lines().last().unwrap_or("").split('|').collect();
    let num = |i: usize| parts.get(i).and_then(|x| x.trim().parse::<u64>().ok()).unwrap_or(0);
    let letter = parts.first().and_then(|x| x.chars().next()).filter(|c| c.is_ascii_alphabetic()).ok_or(format!("could not mount {}: {out}", iso.display()))?;
    Ok((letter, num(1), num(2), num(3)))
}

pub fn dismount_iso(iso: &Path) {
    let p = iso.display().to_string().replace('\'', "''");
    let _ = ps(&format!("Dismount-DiskImage -ImagePath '{p}' | Out-Null"));
}

fn run_diskpart(script: &str, name: &str) -> Result<String, String> {
    let dir = crate::appdirs::home()?.join("install");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let f = dir.join(name);
    std::fs::write(&f, script).map_err(|e| e.to_string())?;
    let out = std::process::Command::new("diskpart").arg("/s").arg(&f).output().map_err(|e| format!("could not run diskpart: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    std::fs::write(dir.join(format!("{name}.log")), &text).ok();
    if !out.status.success() {
        return Err(format!("diskpart reported an error (log: {}):\n{}", dir.join(format!("{name}.log")).display(), text.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")));
    }
    Ok(text)
}

fn bcdedit(args: &[String]) -> Result<String, String> {
    let out = std::process::Command::new("bcdedit").args(args).output().map_err(|e| format!("could not run bcdedit: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        return Err(format!("bcdedit {} failed: {}{}", args.join(" "), text.trim(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(text)
}

fn fresh_hw() -> Result<HardwareFacts, String> {
    let (hw, err) = hardware::scan();
    match err {
        Some(e) => Err(format!("could not re-read the disks: {e}")),
        None => Ok(hw),
    }
}

/// Runs the plan. Assumes every gate already passed and the user typed the
/// confirmation phrase.
pub fn execute(plan: &InstallPlan) -> Result<InstallState, String> {
    let iso = PathBuf::from(&plan.iso);

    // 0. Re-check the disk hasn't changed since the plan was made.
    let hw = fresh_hw()?;
    let c = hw
        .disks
        .iter()
        .find(|d| d.number == plan.disk)
        .and_then(|d| d.partitions.iter().find(|p| p.number == plan.c_partition))
        .ok_or("the C: partition is no longer where the plan expects it; nothing was changed")?;
    if c.drive_letter != Some('C') || c.offset != plan.c_offset || c.size != plan.c_size_before {
        return Err("C: changed since the plan was made; nothing was changed. Run the command again.".into());
    }
    if Path::new(&format!("{}:\\", plan.letter)).exists() {
        return Err(format!("drive letter {}: is now in use; nothing was changed. Run the command again.", plan.letter));
    }

    // 1. Check the ISO's contents BEFORE touching the disk.
    println!("  - Checking the installer files inside the ISO");
    let (iso_letter, iso_files, iso_bytes, largest) = mount_iso(&iso)?;
    if largest >= 4 * install::GB {
        dismount_iso(&iso);
        return Err("the ISO has a file of 4 GB or more, which FAT32 can't hold; nothing was changed. Use a USB stick for this distro.".into());
    }
    if !Path::new(&format!("{iso_letter}:\\EFI\\BOOT\\BOOTX64.EFI")).exists() {
        dismount_iso(&iso);
        return Err("this ISO has no UEFI boot file (EFI\\BOOT\\BOOTX64.EFI); nothing was changed".into());
    }

    let mut state = InstallState { plan: plan.clone(), phase: Phase::Started, bcd_guid: None, started_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string() };
    save_state(&state)?;

    // 2. Shrink C:, create + format the installer partition.
    println!("  - Shrinking C: by {} GB and creating the installer partition (this can take a few minutes)", plan.shrink / install::GB);
    if let Err(e) = run_diskpart(&plan.diskpart_create(), "create.txt") {
        dismount_iso(&iso);
        return Err(format!("{e}\nRun `linuxify install --undo` to put back whatever was changed."));
    }
    let hw2 = fresh_hw()?;
    let new_part = install::find_partition_at(&hw2, plan.disk, plan.installer_offset, plan.installer_size);
    let c_now = hw2.disks.iter().find(|d| d.number == plan.disk).and_then(|d| d.partitions.iter().find(|p| p.drive_letter == Some('C'))).map(|p| p.size);
    if new_part.is_none() || c_now.map(|s| s.abs_diff(plan.c_size_after) > 2 * install::MB).unwrap_or(true) {
        dismount_iso(&iso);
        return Err("after diskpart, the partitions don't look as planned; stopping. Run `linuxify install --undo`.".into());
    }
    state.phase = Phase::Shrunk;
    save_state(&state)?;

    // 3. Copy the ISO's files onto the installer partition and check them.
    println!("  - Copying the installer files ({} files)", iso_files);
    let out = std::process::Command::new("robocopy")
        .args([format!("{iso_letter}:\\"), format!("{}:\\", plan.letter), "/E".into(), "/R:1".into(), "/W:1".into(), "/NFL".into(), "/NDL".into(), "/NP".into()])
        .output()
        .map_err(|e| e.to_string())?;
    dismount_iso(&iso);
    if out.status.code().unwrap_or(16) >= 8 {
        return Err(format!("copying the installer files failed:\n{}\nRun `linuxify install --undo`.", String::from_utf8_lossy(&out.stdout)));
    }
    let (n, bytes) = count_files(&PathBuf::from(format!("{}:\\", plan.letter)));
    if n != iso_files || bytes != iso_bytes {
        return Err(format!("the installer partition has {n} files / {bytes} bytes but the ISO has {iso_files} / {iso_bytes}; stopping. Run `linuxify install --undo`."));
    }
    // Stop Ubuntu-family live systems grabbing the Linux space for persistence.
    for cfg in [r"boot\grub\grub.cfg", r"boot\grub\loopback.cfg"] {
        let p = PathBuf::from(format!(r"{}:\{cfg}", plan.letter));
        if let Ok(text) = std::fs::read_to_string(&p) {
            let (patched, n) = install::patch_casper_grub(&text);
            if n > 0 {
                std::fs::write(&p, patched).map_err(|e| format!("could not update {}: {e}", p.display()))?;
                println!("  - Turned off live-session persistence in {cfg} ({n} boot entries)");
            }
        }
    }
    state.phase = Phase::FilesCopied;
    save_state(&state)?;

    // 4. One-time boot entry.
    println!("  - Adding a one-time boot entry for the installer");
    let created = bcdedit(&["/copy".into(), "{bootmgr}".into(), "/d".into(), install::BCD_DESCRIPTION.into()])?;
    let guid = install::parse_bcd_guid(&created).ok_or(format!("couldn't read the new boot entry id from: {created}"))?;
    state.bcd_guid = Some(guid.clone());
    save_state(&state)?;
    for a in install::bcd_configure_args(&guid, plan.letter) {
        bcdedit(&a)?;
    }
    state.phase = Phase::BootEntryAdded;
    save_state(&state)?;

    // 5. Hide the installer partition from Explorer again.
    let _ = run_diskpart(&format!("select volume {}\r\nremove letter={}\r\nexit\r\n", plan.letter, plan.letter), "hide.txt");
    state.phase = Phase::Ready;
    save_state(&state)?;
    Ok(state)
}

/// Files and bytes under `root`, ignoring "System Volume Information", which
/// Windows creates by itself on every newly mounted volume (e.g. a 12-byte
/// WPSettings.dat): found when testing in a Windows VM.
fn count_files(root: &Path) -> (u64, u64) {
    let svi = root.join("System Volume Information");
    let (mut n, mut bytes) = (0u64, 0u64);
    crate::scan::userdata::walk(root, |p, len| {
        if !p.starts_with(&svi) {
            n += 1;
            bytes += len;
        }
    });
    (n, bytes)
}

/// Undo: remove the boot entry, delete the installer partition, grow C: back.
/// Returns a description of what was done.
pub fn undo(state: &InstallState, dry_run: bool) -> Result<Vec<String>, String> {
    let p = &state.plan;
    let hw = fresh_hw()?;
    let part = install::find_partition_at(&hw, p.disk, p.installer_offset, p.installer_size);
    let mut done = Vec::new();
    if let Some(g) = &state.bcd_guid {
        done.push(format!("bcdedit /delete {g}"));
        if !dry_run {
            let _ = bcdedit(&["/delete".into(), g.clone()]);
        }
    }
    let script = install::diskpart_undo(p.disk, part, p.c_partition);
    done.push(format!("diskpart:\n{}", script.replace("\r\n", "\n    ")));
    if !dry_run {
        run_diskpart(&script, "undo.txt")?;
        let done_path = state_path()?.with_extension("undone.json");
        let _ = std::fs::rename(state_path()?, done_path);
    }
    Ok(done)
}

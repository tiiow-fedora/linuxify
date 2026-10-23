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
pub fn free_letters(hw: &HardwareFacts) -> Vec<char> {
    let used: Vec<char> = hw.disks.iter().flat_map(|d| d.partitions.iter().filter_map(|p| p.drive_letter)).collect();
    "LMNOPQRSTUVWXYZHIJK".chars().filter(|c| !used.contains(c) && !Path::new(&format!("{c}:\\")).exists()).collect()
}

/// Disk number holding a drive letter (None for network paths / unknown).
pub fn disk_of_letter(hw: &HardwareFacts, letter: char) -> Option<u32> {
    let l = letter.to_ascii_uppercase();
    hw.disks.iter().find(|d| d.partitions.iter().any(|p| p.drive_letter == Some(l))).map(|d| d.number)
}

/// Mounts the ISO read-only and returns its drive letter and every file
/// (path relative to the root, size).
pub fn mount_iso(iso: &Path) -> Result<(char, Vec<(String, u64)>), String> {
    let p = iso.display().to_string().replace('\'', "''");
    let out = ps(&format!("$i = Mount-DiskImage -ImagePath '{p}' -Access ReadOnly -PassThru; Start-Sleep 1; ($i | Get-Volume).DriveLetter"))?;
    let letter = out.lines().last().and_then(|x| x.trim().chars().next()).filter(|c| c.is_ascii_alphabetic()).ok_or(format!("could not mount {}: {out}", iso.display()))?;
    let root = PathBuf::from(format!("{letter}:\\"));
    let mut files = Vec::new();
    crate::scan::userdata::walk(&root, |p, len| {
        files.push((p.strip_prefix(&root).unwrap_or(p).display().to_string(), len));
    });
    Ok((letter, files))
}

/// The ISO's files as FAT32 will hold them, plus the case-twins whose contents
/// differ (those can't both survive on FAT32). Leaves the ISO mounted.
pub fn inspect_iso(iso: &Path) -> Result<(char, install::FatView, Vec<String>), String> {
    let (letter, files) = mount_iso(iso)?;
    let view = install::fat_view(&files);
    let mut conflicts = Vec::new();
    for group in &view.case_twins {
        let hashes: Vec<Option<String>> = group.iter().map(|rel| crate::download::sha256_file(&PathBuf::from(format!("{letter}:\\{rel}")), |_| {}).ok()).collect();
        if hashes.iter().any(|h| h.is_none() || *h != hashes[0]) {
            conflicts.push(group.join(" = "));
        }
    }
    Ok((letter, view, conflicts))
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
    let (iso_letter, view, conflicts) = inspect_iso(&iso)?;
    let (iso_files, iso_bytes, largest) = (view.files, view.bytes, view.largest);
    if !conflicts.is_empty() {
        dismount_iso(&iso);
        return Err(format!("the ISO has different files whose names differ only in case ({}), which FAT32 can't hold; nothing was changed. Use a USB stick for this distro.", conflicts.join(", ")));
    }
    if largest > install::FAT32_MAX_FILE && (plan.data_size == 0 || !view.runs_from_data()) {
        dismount_iso(&iso);
        return Err("the ISO has a file of 4 GB or more, which FAT32 can't hold; nothing was changed. Use a USB stick for this distro.".into());
    }
    if let Some(l) = plan.data_letter.filter(|_| plan.data_size > 0) {
        if Path::new(&format!("{l}:\\")).exists() {
            dismount_iso(&iso);
            return Err(format!("drive letter {l}: is now in use; nothing was changed. Run the command again."));
        }
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
    let new_part = install::find_partition_at(&hw2, plan.disk, plan.installer_offset, plan.fat_size());
    let data_ok = plan.data_size == 0 || install::find_partition_at(&hw2, plan.disk, plan.data_offset(), plan.data_size).is_some();
    let c_now = hw2.disks.iter().find(|d| d.number == plan.disk).and_then(|d| d.partitions.iter().find(|p| p.drive_letter == Some('C'))).map(|p| p.size);
    if new_part.is_none() || !data_ok || c_now.map(|s| s.abs_diff(plan.c_size_after) > 2 * install::MB).unwrap_or(true) {
        dismount_iso(&iso);
        return Err("after diskpart, the partitions don't look as planned; stopping. Run `linuxify install --undo`.".into());
    }
    state.phase = Phase::Shrunk;
    save_state(&state)?;

    // 3. Copy the ISO's files onto the installer partition and check them.
    // With LNXDATA: a full copy there, and everything but the big files on LNXINST.
    let data_letter = plan.data_letter.filter(|_| plan.data_size > 0);
    let mut copies = vec![(plan.letter, data_letter.is_some())];
    copies.extend(data_letter.map(|l| (l, false)));
    for (letter, skip_big) in copies {
        println!("  - Copying the installer files to {letter}: ({} files)", if skip_big { iso_files - view.big_files } else { iso_files });
        let mut args = vec![format!("{iso_letter}:\\"), format!("{letter}:\\"), "/E".into(), "/R:1".into(), "/W:1".into(), "/NFL".into(), "/NDL".into(), "/NP".into()];
        if skip_big {
            args.push(format!("/MAX:{}", install::FAT32_MAX_FILE));
        }
        let out = std::process::Command::new("robocopy").args(&args).output().map_err(|e| e.to_string());
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                dismount_iso(&iso);
                return Err(e);
            }
        };
        if out.status.code().unwrap_or(16) >= 8 {
            dismount_iso(&iso);
            return Err(format!("copying the installer files failed:\n{}\nRun `linuxify install --undo`.", String::from_utf8_lossy(&out.stdout)));
        }
        let (want_n, want_bytes) = if skip_big { (iso_files - view.big_files, iso_bytes - view.big_bytes) } else { (iso_files, iso_bytes) };
        let (n, bytes) = count_files(&PathBuf::from(format!("{letter}:\\")));
        if n != want_n || bytes != want_bytes {
            dismount_iso(&iso);
            return Err(format!("{letter}: has {n} files / {bytes} bytes but should have {want_n} / {want_bytes}; stopping. Run `linuxify install --undo`."));
        }
    }
    dismount_iso(&iso);
    // Make the live system run cleanly from LNXINST (see install::adapt_live_boot).
    // Plus systemd-boot entries (Arch's ISO boots with systemd-boot on UEFI).
    // LNXDATA's copy gets the same changes: archiso's GRUB finds its config by
    // a file that's on both partitions, so it may read either one.
    let archiso_label = if data_letter.is_some() { install::DATA_LABEL } else { install::INSTALLER_LABEL };
    for letter in std::iter::once(plan.letter).chain(data_letter) {
        let entries = std::fs::read_dir(format!(r"{letter}:\loader\entries"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.to_ascii_lowercase().ends_with(".conf"))
            .map(|n| format!(r"loader\entries\{n}"));
        for cfg in install::LIVE_BOOT_CONFIGS.iter().map(|c| c.to_string()).chain(entries) {
            let p = PathBuf::from(format!(r"{letter}:\{cfg}"));
            if let Ok(text) = std::fs::read_to_string(&p) {
                let (patched, n) = install::adapt_live_boot_with(&text, install::INSTALLER_LABEL, archiso_label, plan.copy_to_ram);
                if n > 0 {
                    // Files copied off an ISO keep its read-only attribute.
                    if let Ok(meta) = std::fs::metadata(&p) {
                        let mut perms = meta.permissions();
                        #[allow(clippy::permissions_set_readonly_false)]
                        perms.set_readonly(false);
                        let _ = std::fs::set_permissions(&p, perms);
                    }
                    std::fs::write(&p, patched)
                        .map_err(|e| format!("could not update {}: {e}\nRun `linuxify install --undo`.", p.display()))?;
                    println!("  - Adapted {letter}:\\{cfg} to start from the installer partition ({n} lines)");
                }
            }
        }
    }
    if plan.copy_to_ram {
        // Referenced by the adapted dracut boot lines (see install::release_initrd).
        let p = PathBuf::from(format!(r"{}:\{}", plan.letter, install::RELEASE_INITRD));
        std::fs::create_dir_all(p.parent().unwrap_or(&p))
            .and_then(|_| std::fs::write(&p, install::release_initrd()))
            .map_err(|e| format!("could not write {}: {e}\nRun `linuxify install --undo`.", p.display()))?;
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
    for l in std::iter::once(plan.letter).chain(data_letter) {
        let _ = run_diskpart(&format!("select volume {l}\r\nremove letter={l}\r\nexit\r\n"), "hide.txt");
    }
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
    let mut parts: Vec<u32> = install::find_partition_at(&hw, p.disk, p.installer_offset, p.fat_size()).into_iter().collect();
    if p.data_size > 0 {
        parts.extend(install::find_partition_at(&hw, p.disk, p.data_offset(), p.data_size));
    }
    let mut done = Vec::new();
    if let Some(g) = &state.bcd_guid {
        done.push(format!("bcdedit /delete {g}"));
        if !dry_run {
            let _ = bcdedit(&["/delete".into(), g.clone()]);
        }
    }
    let script = install::diskpart_undo(p.disk, &parts, p.c_partition);
    done.push(format!("diskpart:\n{}", script.replace("\r\n", "\n    ")));
    if !dry_run {
        run_diskpart(&script, "undo.txt")?;
        let done_path = state_path()?.with_extension("undone.json");
        let _ = std::fs::rename(state_path()?, done_path);
    }
    // Deleting partitions renumbers the ones after them; re-save the firmware
    // entries (e.g. a Linux installed after LNXINST) so they still start.
    if dry_run {
        done.push("fix the partition numbers in firmware boot entries that moved".into());
    } else {
        match crate::fwboot::refresh_boot_entries(&[p.disk], true) {
            Ok(fixed) => done.extend(fixed),
            Err(e) => eprintln!("  ! couldn't check the firmware boot entries ({e}). If a Linux you installed no longer starts, run `linuxify install --fix-boot-entries`."),
        }
    }
    Ok(done)
}

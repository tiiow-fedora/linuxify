//! The steps the command line (main.rs) and the window (gui/) share: where
//! saved results live, finishing a staged copy, and preparing, confirming and
//! undoing an install. The install gates live here so both front ends go
//! through exactly the same checks before a disk is touched.

use crate::chooser::{self, Answers, Distro, DistroList, Recommendation};
use crate::install::{self, InstallPlan, InstallState};
use crate::{appdirs, download, install_exec, iso, migrate, readiness, report, restore, scan, vm};
use std::path::{Path, PathBuf};

/// The latest saved scan (`linuxify scan`), if any.
pub fn latest_scan() -> Option<scan::ScanReport> {
    let p = appdirs::reports_dir().ok()?.join("latest").join("report.json");
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn choice_json() -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(appdirs::home().ok()?.join("choice.json")).ok()?).ok()
}

/// The top pick from the last `linuxify choose`.
pub fn top_pick() -> Option<String> {
    choice_json()?.pointer("/top/0")?.as_str().map(String::from)
}

/// The answers saved by the last `linuxify choose`.
pub fn saved_answers() -> Option<Answers> {
    serde_json::from_value(choice_json()?.get("answers")?.clone()).ok()
}

/// Saves a recommendation as choice.json + choice.html under %LOCALAPPDATA%\Linuxify
/// (safety rule 1). Returns (json, html) paths.
pub fn save_choice(answers: &Answers, rec: &Recommendation, scan_used: bool) -> Result<(PathBuf, PathBuf), String> {
    let home = appdirs::home()?;
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let saved = serde_json::json!({
        "generated_at": generated_at,
        "scan_used": scan_used,
        "answers": answers,
        "top": rec.ranking.iter().take(3).map(|r| r.id.clone()).collect::<Vec<_>>(),
        "recommendation": rec,
    });
    let json_path = home.join("choice.json");
    let html_path = home.join("choice.html");
    std::fs::write(&json_path, serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::write(&html_path, report::render_choice_html(rec, &generated_at, scan_used)).map_err(|e| e.to_string())?;
    Ok((json_path, html_path))
}

pub fn find_distro<'a>(list: &'a DistroList, id: &str) -> Result<&'a Distro, String> {
    list.distros.iter().find(|d| d.id == id).ok_or_else(|| format!("unknown distro '{id}'. Run `linuxify try --list` to see the ids."))
}

/// File system of the target's drive (e.g. "NTFS", "FAT32"); None for network paths.
pub fn drive_filesystem(p: &Path) -> Option<String> {
    let s = p.to_string_lossy();
    let letter = s.chars().next().filter(|c| c.is_ascii_alphabetic() && s[1..].starts_with(':'))?;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &format!("(Get-Volume -DriveLetter {letter}).FileSystem")])
        .output()
        .ok()?;
    let fs = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!fs.is_empty()).then_some(fs)
}

/// Refuses a copy plan the target can't hold: files of 4 GB or more on FAT32.
pub fn check_target_filesystem(target: &Path, plan: &migrate::Plan) -> Result<(), String> {
    if drive_filesystem(target).as_deref() == Some("FAT32") && plan.files.iter().any(|f| f.size >= 4 * 1024 * 1024 * 1024) {
        return Err("the target drive is FAT32, which can't hold files of 4 GB or more; use an exFAT or NTFS drive".into());
    }
    Ok(())
}

/// Bytes of the plan an earlier, interrupted copy already finished (resume).
pub fn reusable_bytes(plan: &migrate::Plan, staging: &Path) -> u64 {
    let already = std::fs::read_to_string(staging.join(migrate::PROGRESS_FILE)).map(|t| migrate::parse_progress(&t)).unwrap_or_default();
    plan.files.iter().filter(|f| already.get(&f.rel).is_some_and(|c| c.size == f.size && c.mtime == f.mtime)).map(|f| f.size).sum()
}

/// After copying: writes the manifest, the Linux-side scripts, apps.txt and
/// README.txt into the staging folder, and records it in migration.json.
/// Returns the manifest and how many apps went into apps.txt.
pub fn finish_staging(staging: &Path, out: &migrate::CopyOutcome, plan_skipped: &[migrate::Skipped]) -> Result<(migrate::Manifest, usize), String> {
    let mut skipped = plan_skipped.to_vec();
    skipped.extend(out.failed.iter().cloned());
    let manifest = migrate::write_manifest(staging, &out.copied, &skipped)?;
    // Linux side: the scripts must keep Unix line endings.
    std::fs::write(staging.join("restore.sh"), restore::RESTORE_SH.replace("\r\n", "\n")).map_err(|e| e.to_string())?;
    std::fs::write(staging.join("remove-windows.sh"), restore::REMOVE_WINDOWS_SH.replace("\r\n", "\n")).map_err(|e| e.to_string())?;
    let programs = latest_scan().map(|r| r.programs).unwrap_or_default();
    let lines = restore::app_lines(&programs);
    std::fs::write(staging.join("apps.txt"), restore::apps_txt(&lines)).map_err(|e| e.to_string())?;
    let distro = top_pick();
    std::fs::write(staging.join("README.txt"), restore::readme_txt(distro.as_deref(), manifest.total_files, &download::human(manifest.total_bytes))).map_err(|e| e.to_string())?;

    // Remember where the staged copy is (the install and "remove Windows" gates need it).
    let home = appdirs::home()?;
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let record = serde_json::json!({
        "staging": staging.display().to_string(),
        "created_at": manifest.created_at,
        "total_files": manifest.total_files,
        "total_bytes": manifest.total_bytes,
    });
    std::fs::write(home.join("migration.json"), serde_json::to_string_pretty(&record).unwrap()).map_err(|e| e.to_string())?;
    Ok((manifest, lines.len()))
}

/// The staged copy recorded by `linuxify migrate`: (staging folder, files in its manifest).
pub fn recorded_migration() -> Option<(PathBuf, u64)> {
    let rec: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(appdirs::home().ok()?.join("migration.json")).ok()?).ok()?;
    let staging = PathBuf::from(rec.get("staging")?.as_str()?);
    Some((staging, rec.get("total_files").and_then(|v| v.as_u64()).unwrap_or(0)))
}

/// Re-reads every file of the recorded staged copy against its manifest.
pub fn verify_recorded_migration(progress: impl FnMut(u64, u64)) -> Option<(migrate::VerifyReport, PathBuf, u64)> {
    let (staging, total) = recorded_migration()?;
    let r = migrate::verify(&staging, progress).ok()?;
    Some((r, staging, total))
}

/// `%LOCALAPPDATA%\Linuxify\no-install.lock`, when it exists: this PC must never be changed.
pub fn install_lock() -> Option<PathBuf> {
    let lock = appdirs::home().ok()?.join("no-install.lock");
    lock.exists().then_some(lock)
}

fn staging_on_disk(hw: &scan::hardware::HardwareFacts, staging: &Path, disk: Option<u32>) -> bool {
    disk.is_some() && staging.to_string_lossy().chars().next().and_then(|l| install_exec::disk_of_letter(hw, l)) == disk
}

/// Everything known about an install before anything is changed.
pub struct InstallPrep {
    pub distro: Distro,
    pub resolved: download::ResolvedIso,
    pub iso_path: PathBuf,
    pub iso_verified: bool,
    pub is_admin: bool,
    /// Where the verified staged copy is, if there is one.
    pub backup_staging: Option<PathBuf>,
    pub plan: Result<InstallPlan, Vec<String>>,
    /// The plan assumes C: can shrink by its free space (no admin rights to ask Windows).
    pub estimate: bool,
    /// Every reason it must not run; empty means the typed confirmation is all that's left.
    pub problems: Vec<String>,
}

impl InstallPrep {
    /// The plan, when it may actually be carried out after the typed confirmation.
    pub fn runnable(&self) -> Option<&InstallPlan> {
        match &self.plan {
            Ok(p) if self.problems.is_empty() && !self.estimate => Some(p),
            _ => None,
        }
    }
}

/// Resolves the official ISO, re-scans the hardware, re-verifies the staged
/// copy and works out the plan and every gate. Changes nothing (it mounts the
/// verified ISO read-only to look at its files). With `preview`, a non-admin
/// run estimates the shrink size instead of failing.
pub fn prepare_install(
    data_dir: Option<&Path>,
    distro_id: &str,
    linux_gb: Option<u64>,
    preview: bool,
    say: &mut dyn FnMut(&str),
    verify_progress: &mut dyn FnMut(u64, u64),
) -> Result<InstallPrep, String> {
    let (distros, _) = DistroList::load(data_dir)?;
    let home = appdirs::home()?;
    let distro = find_distro(&distros, distro_id)?.clone();
    let isos = iso::IsoList::load(data_dir)?;
    let src = isos.find(&distro.id).ok_or(format!("Linuxify can't fetch a verified ISO for {} yet, so it won't install it. Use a USB stick.", distro.name))?;
    say("Checking the official ISO...");
    let resolved = download::resolve(src)?;
    let iso_path = home.join("isos").join(&resolved.file_name);
    let iso_verified = download::already_verified(&iso_path, &resolved.sha256);
    // Only used before the ISO is downloaded: assume a 6 GB ISO.
    let iso_bytes = std::fs::metadata(&iso_path).map(|m| m.len()).unwrap_or(6 * install::GB);

    // ---- Fresh scan + gates ----
    say("Re-scanning disks and boot settings...");
    let (hw, hw_err) = scan::hardware::scan();
    let findings = readiness::assess(&hw, hw_err.as_deref(), &[], &scan::userdata::UserData::default());
    if let Some((staging, _)) = recorded_migration() {
        say(&format!("Verifying your staged copy in {} (re-reading every file)...", staging.display()));
    }
    let backup = verify_recorded_migration(verify_progress);
    let windows_disk = hw.disks.iter().find(|d| d.partitions.iter().any(|p| p.drive_letter == Some('C'))).map(|d| d.number);
    let backup_ok = backup.as_ref().map(|(r, _, _)| r.missing.is_empty() && r.mismatched.is_empty() && r.ok > 0);
    let backup_on_windows_disk = backup.as_ref().is_some_and(|(_, staging, _)| staging_on_disk(&hw, staging, windows_disk));
    let pre = install::Preflight {
        is_admin: hw.is_admin,
        blockers: install::blockers_from(&findings),
        iso_verified,
        backup_verified: backup_ok,
        backup_on_windows_disk,
        already_installed_state: install_exec::load_state().is_some(),
        secure_boot_on: hw.secure_boot == scan::hardware::SecureBoot::On,
        distro_secure_boot: distro.secure_boot,
        distro_name: distro.name.clone(),
        on_battery: hw.on_battery,
    };
    let mut problems = install::preflight_problems(&pre);
    // A machine owner can lock Linuxify out of changing disks entirely.
    if let Some(lock) = install_lock() {
        problems.insert(0, format!("this PC is locked against installs ({} exists); delete that file only if you really want Linuxify to change this PC's disks", lock.display()));
    }

    // ---- Plan ----
    let letters = install_exec::free_letters(&hw);
    let letter = *letters.first().ok_or("no free drive letter for the installer partition")?;
    // Read-only look inside the ISO, so a file too big for FAT32 shows up in the dry run
    // rather than after the confirmation (Kubuntu 26.04's squashfs is 4.08 GiB).
    let (view, iso_case_conflicts) = if iso_verified {
        say("Looking inside the ISO (read-only)...");
        let r = install_exec::inspect_iso(&iso_path);
        install_exec::dismount_iso(&iso_path);
        r.map(|(_, view, conflicts)| (view, conflicts)).unwrap_or_default()
    } else {
        Default::default()
    };
    let req = install::InstallRequest {
        distro_id: distro.id.clone(),
        iso: iso_path.display().to_string(),
        iso_bytes,
        largest_iso_file: view.largest,
        iso_case_conflicts,
        installer: distro.installer,
        min_cpu_level: distro.min_cpu_level,
        usb_only: distro.usb_only.clone(),
        linux_gb,
        free_letter: letter,
        big_files_bytes: view.big_bytes,
        data_live: view.runs_from_data(),
        data_letter: letters.get(1).copied(),
    };
    let mut plan = install::plan(&hw, &req);
    let mut estimate = false;
    if preview && hw.c_shrinkable.is_none() {
        // Preview only: without admin, assume C: can shrink by its free space.
        let mut guess = hw.clone();
        guess.c_shrinkable = hw.disks.iter().flat_map(|d| d.partitions.iter()).find(|p| p.drive_letter == Some('C')).and_then(|p| p.free);
        plan = install::plan(&guess, &req);
        estimate = true;
    }
    Ok(InstallPrep {
        distro,
        resolved,
        iso_path,
        iso_verified,
        is_admin: hw.is_admin,
        backup_staging: backup.filter(|(r, _, _)| r.missing.is_empty() && r.mismatched.is_empty()).map(|(_, s, _)| s),
        plan,
        estimate,
        problems,
    })
}

/// The plan in plain words, one entry per step (the dry run shows these).
pub fn plan_steps(p: &InstallPlan, distro_name: &str) -> Vec<String> {
    let mut v = vec![
        format!("Shrink C: from {} GB to {} GB (frees {} GB). Windows and your files stay.", p.c_size_before / install::GB, p.c_size_after / install::GB, p.shrink / install::GB),
        format!("Create a {} GB FAT32 partition '{}' right after C: and copy the verified {distro_name} installer onto it.", p.fat_size() / install::GB, install::INSTALLER_LABEL),
    ];
    if p.data_size > 0 {
        v[1] += &format!(
            " Its largest file is too big for FAT32, so a {} GB NTFS partition '{}' after it gets a full copy of the installer, which the live system starts from.",
            p.data_size / install::GB,
            install::DATA_LABEL
        );
    }
    v.push(format!("Leave {} GB unallocated for Linux.", p.linux_size / install::GB));
    v.push("Add a ONE-TIME boot entry so the next restart starts the installer (later restarts go to Windows as usual).".into());
    v
}

/// The exact commands behind [`plan_steps`].
pub fn plan_commands(p: &InstallPlan) -> String {
    format!(
        "diskpart script:\n  {}\nthen: robocopy <ISO> {}:\\ ; bcdedit /copy {{bootmgr}} /d \"{}\" ; bcdedit /set <id> device partition={}: ; bcdedit /set <id> path \\EFI\\BOOT\\BOOTX64.EFI ; bcdedit /set {{fwbootmgr}} bootsequence <id> ; diskpart remove letter={}",
        p.diskpart_create().trim_end().replace("\r\n", "\n  "),
        p.letter,
        install::BCD_DESCRIPTION,
        p.letter,
        p.letter
    )
}

/// The last gate before the disk is changed: refuses unless the preparation
/// found no problem, the plan is exact (not an estimate), the PC isn't locked,
/// and `typed` is exactly the plan's confirmation phrase. Only then runs it.
pub fn install_confirmed(prep: &InstallPrep, typed: &str) -> Result<InstallState, String> {
    let plan = prep.plan.as_ref().map_err(|w| format!("Linuxify can't make a plan: {}; nothing was changed", w.join("; ")))?;
    check_install_gates(prep, typed)?;
    install_exec::execute(plan)
}

/// The checks of [`install_confirmed`] without running anything.
pub fn check_install_gates(prep: &InstallPrep, typed: &str) -> Result<(), String> {
    let plan = prep.plan.as_ref().map_err(|w| w.join("; "))?;
    if !prep.problems.is_empty() {
        return Err("not safe to continue; nothing was changed".into());
    }
    if prep.estimate {
        return Err("the plan is only an estimate (no administrator rights); nothing was changed".into());
    }
    if let Some(lock) = install_lock() {
        return Err(format!("this PC is locked against disk changes ({} exists); nothing was changed", lock.display()));
    }
    if typed.trim() != plan.confirm_phrase() {
        return Err("confirmation didn't match; nothing was changed".into());
    }
    Ok(())
}

/// What the undo of `state` will change, in words.
pub fn undo_summary(state: &InstallState) -> String {
    let p = &state.plan;
    let parts = if p.data_size > 0 {
        format!("{} and {} installer partitions", install::INSTALLER_LABEL, install::DATA_LABEL)
    } else {
        format!("{} installer partition", install::INSTALLER_LABEL)
    };
    format!(
        "Undo will: delete the {} boot entry, delete the {parts} (disk {}, at {} GB), and grow C: back into the free space.",
        install::BCD_DESCRIPTION,
        p.disk,
        p.installer_offset / install::GB
    )
}

pub fn undo_phrase(state: &InstallState) -> String {
    format!("UNDO LINUXIFY ON DISK {}", state.plan.disk)
}

/// Runs the undo only when the PC isn't locked and `typed` matches [`undo_phrase`].
pub fn undo_confirmed(state: &InstallState, typed: &str) -> Result<Vec<String>, String> {
    if let Some(lock) = install_lock() {
        return Err(format!("this PC is locked against disk changes ({} exists); nothing was changed", lock.display()));
    }
    if typed.trim() != undo_phrase(state) {
        return Err("confirmation didn't match; nothing was changed".into());
    }
    install_exec::undo(state, false)
}

/// Windows-side half of "remove Windows": verifies the staged copy completely
/// (count + hashes) and checks it doesn't live on the Windows disk. Returns the
/// gate and every reason it stays locked. It never deletes anything.
pub fn replace_windows_gate(progress: impl FnMut(u64, u64)) -> (install::ReplaceGate, Vec<String>) {
    let (hw, _) = scan::hardware::scan();
    let windows_disk = hw.disks.iter().find(|d| d.partitions.iter().any(|p| p.drive_letter == Some('C'))).map(|d| d.number);
    let gate = match verify_recorded_migration(progress) {
        None => install::ReplaceGate::default(),
        Some((r, staging, total)) => install::ReplaceGate {
            migration_recorded: true,
            staging_files_ok: r.ok,
            staging_missing: r.missing.len() as u64,
            staging_mismatched: r.mismatched.len() as u64,
            manifest_files: total,
            staging_on_windows_disk: staging_on_disk(&hw, &staging, windows_disk),
        },
    };
    let problems = install::replace_gate_problems(&gate);
    (gate, problems)
}

/// The VM program to use: the one asked for, else VirtualBox, Hyper-V, QEMU
/// (the first that works).
pub fn pick_backend<'a>(backends: &'a [vm::Detected], want: Option<&str>) -> Result<&'a vm::Detected, String> {
    let chosen = match want {
        Some(name) => {
            let want = vm::Backend::parse(name).ok_or_else(|| format!("unknown VM program '{name}' (use virtualbox, hyperv or qemu)"))?;
            backends.iter().find(|b| b.backend == want).ok_or_else(|| format!("{} was not found on this PC", want.label()))?
        }
        None => [vm::Backend::VirtualBox, vm::Backend::HyperV, vm::Backend::Qemu]
            .iter()
            .find_map(|w| backends.iter().find(|b| b.backend == *w && b.unusable.is_none()))
            .ok_or_else(|| {
                "no usable VM program found. The easiest fix is `winget install Oracle.VirtualBox` (free), then run this again.                  Alternatives: QEMU (`winget install SoftwareFreedomConservancy.QEMU`) or Hyper-V from an administrator terminal."
                    .to_string()
            })?,
    };
    match &chosen.unusable {
        Some(u) => Err(u.clone()),
        None => Ok(chosen),
    }
}

/// A trial VM sized for this PC (from the latest scan).
pub fn trial_spec(distro_id: &str, iso: PathBuf, disk_gb: Option<u32>) -> Result<vm::TrialSpec, String> {
    let scan_report = latest_scan();
    let (memory_mb, cpus) = vm::size_for_host(
        scan_report.as_ref().and_then(|r| r.hardware.ram_bytes).map(|b| b as f64 / (1u64 << 30) as f64),
        scan_report.as_ref().and_then(|r| r.hardware.cpus.first()).and_then(|c| c.threads),
    );
    Ok(vm::TrialSpec { distro_id: distro_id.to_string(), iso, memory_mb, cpus, disk_gb, vm_dir: appdirs::home()?.join("vms") })
}

/// The commands that create (or reuse) and start the trial VM.
pub fn trial_steps(b: &vm::Detected, spec: &vm::TrialSpec, dry_run: bool) -> Vec<vm::Step> {
    match b.backend {
        vm::Backend::VirtualBox => vm::plan_virtualbox(&b.exe, spec, !dry_run && vm::vbox_vm_exists(&b.exe, &spec.distro_id)),
        vm::Backend::Qemu => vm::plan_qemu(&b.exe, spec),
        vm::Backend::HyperV => vm::plan_hyperv(&b.exe, spec, !dry_run && vm::hyperv_vm_exists(&spec.distro_id)),
    }
}

/// Re-ranks saved or new answers against the latest scan.
pub fn rank_with_scan(list: &DistroList, answers: &Answers, scan_report: Option<&scan::ScanReport>) -> Recommendation {
    let hints = scan_report.map(chooser::hints_from_scan);
    chooser::rank(list, answers, hints.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> InstallPlan {
        InstallPlan {
            distro_id: "linux-mint".into(),
            iso: "x.iso".into(),
            disk: 0,
            c_partition: 3,
            c_offset: 0,
            c_size_before: 200 * install::GB,
            shrink: 105 * install::GB,
            c_size_after: 95 * install::GB,
            installer_offset: 95 * install::GB,
            installer_size: 5 * install::GB,
            linux_size: 100 * install::GB,
            letter: 'L',
            copy_to_ram: false,
            disk_size: 256 * install::GB,
            data_size: 0,
            data_letter: None,
        }
    }

    fn prep(problems: Vec<String>, estimate: bool) -> InstallPrep {
        InstallPrep {
            distro: DistroList::parse(chooser::EMBEDDED_DISTROS).unwrap().distros[0].clone(),
            resolved: download::ResolvedIso { file_name: "x.iso".into(), sha256: "00".into(), url: String::new(), checksum_url: String::new() },
            iso_path: "x.iso".into(),
            iso_verified: true,
            is_admin: true,
            backup_staging: None,
            plan: Ok(plan()),
            estimate,
            problems,
        }
    }

    // These never reach install_exec::execute: every case is refused first.
    #[test]
    fn gates_refuse_problems_estimates_and_wrong_phrases() {
        let phrase = plan().confirm_phrase();
        assert!(check_install_gates(&prep(vec!["readiness blocker: x".into()], false), &phrase).is_err());
        assert!(check_install_gates(&prep(vec![], true), &phrase).is_err(), "an estimate never runs");
        assert!(check_install_gates(&prep(vec![], false), "SHRINK C: ON DISK 0 BY 104 GB").is_err());
        assert!(check_install_gates(&prep(vec![], false), "").is_err());
        let mut failed = prep(vec![], false);
        failed.plan = Err(vec!["not enough space".into()]);
        assert!(check_install_gates(&failed, &phrase).is_err());
        assert!(install_confirmed(&failed, &phrase).is_err());
        assert!(install_confirmed(&prep(vec!["x".into()], false), &phrase).is_err());
    }

    #[test]
    fn runnable_only_without_problems_or_estimate() {
        assert!(prep(vec![], false).runnable().is_some());
        assert!(prep(vec![], true).runnable().is_none());
        assert!(prep(vec!["x".into()], false).runnable().is_none());
    }

    #[test]
    fn undo_needs_the_exact_phrase() {
        let state = InstallState { plan: plan(), phase: install::Phase::Ready, bcd_guid: None, started_at: String::new() };
        assert_eq!(undo_phrase(&state), "UNDO LINUXIFY ON DISK 0");
        assert!(undo_confirmed(&state, "UNDO LINUXIFY ON DISK 1").is_err());
        assert!(undo_confirmed(&state, "undo linuxify on disk 0").is_err());
    }

    #[test]
    fn plan_steps_mention_the_data_partition_only_when_there_is_one() {
        let mut p = plan();
        assert_eq!(plan_steps(&p, "Mint").len(), 4);
        assert!(!plan_steps(&p, "Mint")[1].contains(install::DATA_LABEL));
        p.data_size = 5 * install::GB;
        p.installer_size = 10 * install::GB;
        p.data_letter = Some('M');
        assert!(plan_steps(&p, "Kubuntu")[1].contains(install::DATA_LABEL));
        assert!(plan_commands(&p).contains("format quick fs=ntfs"));
    }
}

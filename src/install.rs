//! Stage 5: prepare a dual-boot install.
//!
//! What it does, in order (each step printed by --dry-run first):
//!   1. shrink C: by (installer + Linux) space,
//!   2. create a small FAT32 "LNXINST" partition right after C: and copy the
//!      verified ISO's files onto it (UEFI firmware can boot it directly),
//!   3. leave the rest unallocated for the Linux installer's
//!      "install alongside Windows" option,
//!   4. add a ONE-TIME UEFI boot entry for the installer partition.
//!
//! Windows stays. Everything is recorded in install-state.json so `--undo`
//! can put it back.
//!
//! This module is pure planning + safety gates (unit-tested). The thin
//! executor lives in main.rs and only runs after an exact typed confirmation.

use crate::readiness::{Finding, Severity, ESP_GUID};
use crate::scan::hardware::{Firmware, HardwareFacts};
use serde::{Deserialize, Serialize};

pub const GB: u64 = 1024 * 1024 * 1024;
pub const MB: u64 = 1024 * 1024;
pub const INSTALLER_LABEL: &str = "LNXINST";
pub const BCD_DESCRIPTION: &str = "Linuxify Linux installer";
/// Never leave Windows with less free space than this after shrinking.
pub const KEEP_WINDOWS_FREE: u64 = 20 * GB;
/// Smallest Linux space Linuxify will set up.
pub const MIN_LINUX: u64 = 40 * GB;
pub const DEFAULT_LINUX: u64 = 100 * GB;
const FAT32_MAX_FILE: u64 = 4 * GB - 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallPlan {
    pub distro_id: String,
    pub iso: String,
    pub disk: u32,
    pub c_partition: u32,
    pub c_offset: u64,
    pub c_size_before: u64,
    pub shrink: u64,
    pub c_size_after: u64,
    pub installer_offset: u64,
    pub installer_size: u64,
    pub linux_size: u64,
    pub letter: char,
}

impl InstallPlan {
    /// The exact words the user must type to go ahead.
    pub fn confirm_phrase(&self) -> String {
        format!("SHRINK C: ON DISK {} BY {} GB", self.disk, self.shrink / GB)
    }

    /// diskpart script for steps 1-2 (shrink, create, format, temporary letter).
    /// The new partition is placed at an explicit offset (right after the
    /// shrunk C:), so diskpart can't pick some other free area.
    pub fn diskpart_create(&self) -> String {
        format!(
            "rem Linuxify: shrink C: and create the installer partition\r\n\
             select disk {disk}\r\n\
             select partition {part}\r\n\
             shrink desired={shrink_mb} minimum={shrink_mb}\r\n\
             create partition primary size={inst_mb} offset={offset_kb}\r\n\
             format quick fs=fat32 label={label}\r\n\
             assign letter={letter}\r\n\
             exit\r\n",
            disk = self.disk,
            part = self.c_partition,
            shrink_mb = self.shrink / MB,
            inst_mb = self.installer_size / MB,
            offset_kb = self.installer_offset / 1024,
            label = INSTALLER_LABEL,
            letter = self.letter,
        )
    }
}

/// What the user can ask for.
#[derive(Debug, Clone)]
pub struct InstallRequest {
    pub distro_id: String,
    pub iso: String,
    pub iso_bytes: u64,
    pub largest_iso_file: u64,
    /// Linux space in GB (None = default 100 GB, or as much as fits).
    pub linux_gb: Option<u64>,
    pub free_letter: char,
}

fn round_up(x: u64, unit: u64) -> u64 {
    x.div_ceil(unit) * unit
}

/// Sizes the installer partition: ISO + 10% + 512 MB, rounded up to 1 GB, at least 4 GB.
pub fn installer_size_for(iso_bytes: u64) -> u64 {
    round_up(iso_bytes + iso_bytes / 10 + 512 * MB, GB).max(4 * GB)
}

/// Builds the plan, or explains every reason it can't.
pub fn plan(hw: &HardwareFacts, req: &InstallRequest) -> Result<InstallPlan, Vec<String>> {
    let mut why = Vec::new();
    if hw.firmware != Firmware::Uefi {
        why.push("Linuxify only prepares installs on UEFI PCs (this one isn't, or it couldn't tell). Use a USB stick instead.".to_string());
    }
    if req.largest_iso_file > FAT32_MAX_FILE {
        why.push(format!(
            "this ISO contains a file over 4 GB ({} GB), which can't go on a FAT32 installer partition. Use a USB stick for this distro.",
            req.largest_iso_file / GB
        ));
    }
    let Some((disk, part)) = hw.disks.iter().find_map(|d| d.partitions.iter().find(|p| p.drive_letter == Some('C')).map(|p| (d, p))) else {
        why.push("couldn't find the C: partition".into());
        return Err(why);
    };
    if disk.partition_style.as_deref() != Some("GPT") {
        why.push(format!("disk {} uses {} partitioning; Linuxify only handles GPT disks", disk.number, disk.partition_style.as_deref().unwrap_or("unknown")));
    }
    if !disk.partitions.iter().any(|p| p.gpt_type.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(ESP_GUID))) && !hw.disks.iter().any(|d| d.partitions.iter().any(|p| p.gpt_type.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(ESP_GUID)))) {
        why.push("no EFI System Partition found".into());
    }
    let Some(shrinkable) = hw.c_shrinkable else {
        why.push("the exact amount C: can shrink by is unknown (run Linuxify as administrator)".into());
        return Err(why);
    };
    let c_free = part.free.unwrap_or(0);
    let installer = installer_size_for(req.iso_bytes);
    // Cap by what Windows says it can shrink, and keep 20 GB free for Windows.
    let max_total = shrinkable.min(c_free.saturating_sub(KEEP_WINDOWS_FREE));
    let max_linux = round_down_gb(max_total.saturating_sub(installer));
    let linux = match req.linux_gb {
        Some(g) => g * GB,
        None => DEFAULT_LINUX.min(max_linux),
    };
    if linux < MIN_LINUX {
        why.push(format!("Linux needs at least {} GB but only {} GB can be freed safely", MIN_LINUX / GB, max_linux / GB));
    }
    if linux > max_linux {
        why.push(format!("{} GB was requested for Linux but at most {} GB can be freed safely", linux / GB, max_linux / GB));
    }
    if !why.is_empty() {
        return Err(why);
    }
    let shrink = linux + installer;
    let c_after = part.size - shrink;
    Ok(InstallPlan {
        distro_id: req.distro_id.clone(),
        iso: req.iso.clone(),
        disk: disk.number,
        c_partition: part.number,
        c_offset: part.offset,
        c_size_before: part.size,
        shrink,
        c_size_after: c_after,
        installer_offset: part.offset + c_after,
        installer_size: installer,
        linux_size: linux,
        letter: req.free_letter,
    })
}

fn round_down_gb(x: u64) -> u64 {
    x / GB * GB
}

// ---------------------------------------------------------------------------
// Safety gates
// ---------------------------------------------------------------------------

/// Everything that must be true before Linuxify changes a disk.
#[derive(Debug, Clone, Default)]
pub struct Preflight {
    pub is_admin: bool,
    /// From readiness::assess on a FRESH scan (not an old report).
    pub blockers: Vec<String>,
    pub iso_verified: bool,
    /// migrate --verify passed just now ("verified backup check").
    pub backup_verified: Option<bool>,
    /// The staged copy lives on the disk that's about to be repartitioned.
    pub backup_on_windows_disk: bool,
    pub already_installed_state: bool,
}

pub fn blockers_from(findings: &[Finding]) -> Vec<String> {
    findings.iter().filter(|f| f.severity == Severity::Blocker).map(|f| f.title.clone()).collect()
}

/// Returns every reason the install must not proceed (empty = OK).
pub fn preflight_problems(p: &Preflight) -> Vec<String> {
    let mut v = Vec::new();
    if !p.is_admin {
        v.push("Linuxify must run from an administrator terminal to change disks".into());
    }
    for b in &p.blockers {
        v.push(format!("readiness blocker: {b}"));
    }
    if !p.iso_verified {
        v.push("the installer ISO hasn't been downloaded and verified (run `linuxify try <distro>` first, or check the ISO)".into());
    }
    match p.backup_verified {
        None => v.push("no verified copy of your files: run `linuxify migrate --target <drive>` first (to a different drive than C:)".into()),
        Some(false) => v.push("your staged copy (linuxify migrate) failed verification; fix it before installing".into()),
        Some(true) => {}
    }
    if p.backup_on_windows_disk {
        v.push("your staged copy is on the same disk as Windows (the one being changed); stage it to a different drive, USB disk or NAS".into());
    }
    if p.already_installed_state {
        v.push("a previous Linuxify install setup is still in place; run `linuxify install --undo` first or finish that install".into());
    }
    v
}

/// The Windows-side half of the --replace-windows gate. Removing Windows
/// itself happens later, from Linux, by remove-windows.sh, which re-checks
/// the restored files. This only ever unlocks those instructions.
#[derive(Debug, Clone, Default)]
pub struct ReplaceGate {
    pub migration_recorded: bool,
    pub staging_files_ok: u64,
    pub staging_missing: u64,
    pub staging_mismatched: u64,
    pub manifest_files: u64,
    /// The staging folder is on the same physical disk as Windows (it would be erased with it).
    pub staging_on_windows_disk: bool,
}

pub fn replace_gate_problems(g: &ReplaceGate) -> Vec<String> {
    let mut v = Vec::new();
    if !g.migration_recorded {
        v.push("no migration found: run `linuxify migrate --target <drive>` first".into());
        return v;
    }
    if g.manifest_files == 0 {
        v.push("the migration manifest lists no files".into());
    }
    if g.staging_missing > 0 || g.staging_mismatched > 0 {
        v.push(format!("{} staged files are missing and {} don't match their checksums", g.staging_missing, g.staging_mismatched));
    }
    if g.staging_files_ok != g.manifest_files {
        v.push(format!("file count check failed: {} of {} files verified", g.staging_files_ok, g.manifest_files));
    }
    if g.staging_on_windows_disk {
        v.push("your staged copy is on the same disk as Windows, so removing Windows could destroy it; stage to another drive".into());
    }
    v
}

// ---------------------------------------------------------------------------
// Boot entry + undo
// ---------------------------------------------------------------------------

/// Pulls the {GUID} out of `bcdedit /copy` output (which is localised, so
/// only the GUID pattern is trusted).
pub fn parse_bcd_guid(output: &str) -> Option<String> {
    let start = output.find('{')?;
    let end = output[start..].find('}')? + start;
    let g = &output[start..=end];
    let inner = &g[1..g.len() - 1];
    let ok = inner.len() == 36 && inner.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
    ok.then(|| g.to_string())
}

/// bcdedit arguments after the entry has been created as a copy of {bootmgr}.
pub fn bcd_configure_args(guid: &str, letter: char) -> Vec<Vec<String>> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    vec![
        s(&["/set", guid, "device", &format!("partition={letter}:")]),
        s(&["/set", guid, "path", r"\EFI\BOOT\BOOTX64.EFI"]),
        // One-time: only the NEXT restart goes to the installer; after that
        // the PC boots as before.
        s(&["/set", "{fwbootmgr}", "bootsequence", guid]),
    ]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Written before the first change, so a crash leaves a trace.
    Started,
    Shrunk,
    FilesCopied,
    BootEntryAdded,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallState {
    pub plan: InstallPlan,
    pub phase: Phase,
    pub bcd_guid: Option<String>,
    pub started_at: String,
}

/// diskpart script that deletes the installer partition (found by its
/// recorded offset at undo time) and grows C: back into the free space.
pub fn diskpart_undo(disk: u32, installer_partition_number: Option<u32>, c_partition: u32) -> String {
    let mut s = format!("rem Linuxify: undo\r\nselect disk {disk}\r\n");
    if let Some(n) = installer_partition_number {
        // No "override": diskpart then refuses to delete protected/system partitions.
        s += &format!("select partition {n}\r\ndelete partition\r\n");
    }
    s += &format!("select partition {c_partition}\r\nextend\r\nexit\r\n");
    s
}

/// Finds the partition number now sitting at `offset` with the expected size
/// (partition numbers can change, offsets don't).
pub fn find_partition_at(hw: &HardwareFacts, disk: u32, offset: u64, size: u64) -> Option<u32> {
    hw.disks.iter().find(|d| d.number == disk)?.partitions.iter().find(|p| p.offset == offset && p.size.abs_diff(size) < 2 * MB).map(|p| p.number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::hardware::*;
    use std::collections::BTreeMap;

    const BASIC: &str = "{ebd0a0a2-b9e5-4433-87c0-68b6b72699c7}";

    fn part(n: u32, letter: Option<char>, gpt: &str, offset: u64, size: u64, free: Option<u64>) -> Partition {
        Partition { number: n, drive_letter: letter, size, offset, gpt_type: Some(gpt.into()), free, ..Default::default() }
    }

    /// A typical single-disk UEFI PC: ESP, MSR, C: (476 GB, 300 GB free), recovery.
    fn hw() -> HardwareFacts {
        HardwareFacts {
            is_admin: true,
            os: None,
            cpus: vec![],
            ram_bytes: Some(16 * GB),
            computer: None,
            gpus: vec![],
            disks: vec![Disk {
                number: 0,
                model: Some("NVMe".into()),
                size: 512 * GB,
                bus_type: Some("NVMe".into()),
                partition_style: Some("GPT".into()),
                is_boot: true,
                is_system: true,
                unallocated: 0,
                partitions: vec![
                    part(1, None, ESP_GUID, MB, 100 * MB, None),
                    part(2, None, "{e3c9e316-0b5c-4db8-817d-f92df00215ae}", 101 * MB, 16 * MB, None),
                    part(3, Some('C'), BASIC, 117 * MB, 476 * GB, Some(300 * GB)),
                    part(4, None, "{de94bba4-06d1-4d40-a16a-bfd50179d6ac}", 117 * MB + 476 * GB, 700 * MB, None),
                ],
            }],
            storage_controllers: vec![],
            wifi: vec![],
            firmware: Firmware::Uefi,
            secure_boot: SecureBoot::On,
            bitlocker: vec![],
            rst_vmd: RstVmd { detected: false, evidence: vec![] },
            c_drive: None,
            c_shrinkable: Some(250 * GB),
            fast_startup: Some(false),
            probe_errors: BTreeMap::new(),
        }
    }

    fn req() -> InstallRequest {
        InstallRequest { distro_id: "linux-mint".into(), iso: "mint.iso".into(), iso_bytes: 3_091_660_800, largest_iso_file: 2_900_000_000, linux_gb: None, free_letter: 'L' }
    }

    #[test]
    fn default_plan_layout() {
        let p = plan(&hw(), &req()).unwrap();
        assert_eq!(p.disk, 0);
        assert_eq!(p.c_partition, 3);
        assert_eq!(p.installer_size, 4 * GB); // 2.88 GB + 10% + 0.5 GB = 3.67 GB, rounded up
        assert_eq!(p.linux_size, 100 * GB);
        assert_eq!(p.shrink, 104 * GB);
        assert_eq!(p.c_size_after, 476 * GB - 104 * GB);
        // Installer starts exactly where the shrunk C: ends; Linux space follows it.
        assert_eq!(p.installer_offset, p.c_offset + p.c_size_after);
        assert!(p.installer_offset + p.installer_size + p.linux_size <= p.c_offset + p.c_size_before);
        assert_eq!(p.confirm_phrase(), "SHRINK C: ON DISK 0 BY 104 GB");
    }

    #[test]
    fn diskpart_script_is_exact() {
        let p = plan(&hw(), &req()).unwrap();
        let s = p.diskpart_create();
        assert!(s.contains("select disk 0\r\nselect partition 3\r\n"));
        assert!(s.contains(&format!("shrink desired={0} minimum={0}\r\n", 104 * 1024)));
        // Must wait for the shrink to finish before creating the partition.
        assert!(!s.contains("nowait"));
        assert!(s.contains(&format!("create partition primary size={} offset={}", 4 * 1024, p.installer_offset / 1024)));
        assert!(s.contains("format quick fs=fat32 label=LNXINST"));
        assert!(s.contains("assign letter=L"));
        // Never anything that wipes a disk.
        for bad in ["clean", "delete", "convert", "override"] {
            assert!(!s.to_lowercase().contains(bad), "create script must not contain '{bad}'");
        }
    }

    #[test]
    fn keeps_windows_free_space_and_respects_shrink_limit() {
        let mut h = hw();
        // C: has only 70 GB free: 70 - 20 kept = 50 -> 46 for Linux after a 4 GB installer.
        h.disks[0].partitions[2].free = Some(70 * GB);
        let p = plan(&h, &req()).unwrap();
        assert_eq!(p.linux_size, 46 * GB);
        // Windows says it can only shrink 30 GB (unmovable files): refuse.
        h.c_shrinkable = Some(30 * GB);
        let e = plan(&h, &req()).unwrap_err();
        assert!(e[0].contains("at least 40 GB"), "{e:?}");
    }

    #[test]
    fn explicit_size_too_big_is_refused() {
        let mut r = req();
        r.linux_gb = Some(400);
        let e = plan(&hw(), &r).unwrap_err();
        assert!(e.iter().any(|x| x.contains("400 GB was requested")), "{e:?}");
    }

    #[test]
    fn refuses_bios_mbr_unknown_shrink_and_big_files() {
        let mut h = hw();
        h.firmware = Firmware::Bios;
        h.disks[0].partition_style = Some("MBR".into());
        let mut r = req();
        r.largest_iso_file = 5 * GB;
        let e = plan(&h, &r).unwrap_err();
        assert!(e.iter().any(|x| x.contains("UEFI")));
        assert!(e.iter().any(|x| x.contains("GPT")));
        assert!(e.iter().any(|x| x.contains("over 4 GB")));
        let mut h2 = hw();
        h2.c_shrinkable = None;
        assert!(plan(&h2, &req()).unwrap_err().iter().any(|x| x.contains("administrator")));
    }

    #[test]
    fn installer_sizing() {
        assert_eq!(installer_size_for(1_500_000_000), 4 * GB);
        assert_eq!(installer_size_for(3_091_660_800), 4 * GB);
        assert_eq!(installer_size_for(6 * GB), 8 * GB);
    }

    #[test]
    fn preflight_requires_everything() {
        let ok = Preflight { is_admin: true, blockers: vec![], iso_verified: true, backup_verified: Some(true), backup_on_windows_disk: false, already_installed_state: false };
        assert!(preflight_problems(&ok).is_empty());
        let bad = Preflight { is_admin: false, blockers: vec!["BitLocker is on for C:".into()], iso_verified: false, backup_verified: None, backup_on_windows_disk: true, already_installed_state: true };
        let p = preflight_problems(&bad);
        assert_eq!(p.len(), 6, "{p:?}");
        let failed_backup = Preflight { backup_verified: Some(false), ..ok.clone() };
        assert!(preflight_problems(&failed_backup)[0].contains("failed verification"));
        let same_disk = Preflight { backup_on_windows_disk: true, ..ok };
        assert!(preflight_problems(&same_disk)[0].contains("same disk"));
    }

    #[test]
    fn replace_gate_refuses_unless_everything_verifies() {
        let good = ReplaceGate { migration_recorded: true, staging_files_ok: 2013, staging_missing: 0, staging_mismatched: 0, manifest_files: 2013, staging_on_windows_disk: false };
        assert!(replace_gate_problems(&good).is_empty());
        // One damaged file.
        let damaged = ReplaceGate { staging_files_ok: 2012, staging_mismatched: 1, ..good.clone() };
        let p = replace_gate_problems(&damaged);
        assert!(p.iter().any(|x| x.contains("don't match")) && p.iter().any(|x| x.contains("2012 of 2013")), "{p:?}");
        // One missing file.
        let missing = ReplaceGate { staging_files_ok: 2012, staging_missing: 1, ..good.clone() };
        assert!(!replace_gate_problems(&missing).is_empty());
        // Copy lives on the disk that would be wiped.
        let same_disk = ReplaceGate { staging_on_windows_disk: true, ..good.clone() };
        assert!(replace_gate_problems(&same_disk)[0].contains("same disk"));
        // No migration at all.
        assert!(replace_gate_problems(&ReplaceGate::default())[0].contains("no migration"));
        // Empty manifest.
        let empty = ReplaceGate { staging_files_ok: 0, manifest_files: 0, ..good };
        assert!(!replace_gate_problems(&empty).is_empty());
    }

    #[test]
    fn bcd_guid_parsing() {
        assert_eq!(parse_bcd_guid("The entry was successfully copied to {3c1a4b2e-1d2c-11ef-9a8b-0123456789ab}."), Some("{3c1a4b2e-1d2c-11ef-9a8b-0123456789ab}".into()));
        assert_eq!(parse_bcd_guid("Der Eintrag wurde erfolgreich in {3c1a4b2e-1d2c-11ef-9a8b-0123456789ab} kopiert."), Some("{3c1a4b2e-1d2c-11ef-9a8b-0123456789ab}".into()));
        assert_eq!(parse_bcd_guid("{bootmgr}"), None);
        assert_eq!(parse_bcd_guid("error"), None);
    }

    #[test]
    fn bcd_args_are_one_time_and_scoped() {
        let a = bcd_configure_args("{3c1a4b2e-1d2c-11ef-9a8b-0123456789ab}", 'L');
        assert_eq!(a[0], ["/set", "{3c1a4b2e-1d2c-11ef-9a8b-0123456789ab}", "device", "partition=L:"]);
        assert_eq!(a[2], ["/set", "{fwbootmgr}", "bootsequence", "{3c1a4b2e-1d2c-11ef-9a8b-0123456789ab}"]);
        // Never touches the default boot order or Windows' own entry.
        assert!(!a.iter().flatten().any(|x| x == "displayorder" || x == "{current}" || x == "{default}"));
    }

    #[test]
    fn undo_script_and_partition_lookup() {
        let s = diskpart_undo(0, Some(4), 3);
        assert!(s.contains("select partition 4\r\ndelete partition\r\nselect partition 3\r\nextend"));
        assert!(!s.contains("override"), "undo must not force-delete protected partitions");
        let s2 = diskpart_undo(0, None, 3);
        assert!(!s2.contains("delete"));
        let mut h = hw();
        let p = plan(&h, &req()).unwrap();
        h.disks[0].partitions.push(part(5, Some('L'), BASIC, p.installer_offset, p.installer_size, None));
        assert_eq!(find_partition_at(&h, 0, p.installer_offset, p.installer_size), Some(5));
        assert_eq!(find_partition_at(&h, 0, p.installer_offset + GB, p.installer_size), None);
    }

    #[test]
    fn plan_on_this_machine_layout_esp_on_other_disk() {
        // Developer PC: C: alone on disk 0 (MSR + C:), ESP on disk 1.
        let mut h = hw();
        h.disks[0].partitions.retain(|p| p.number != 1);
        h.disks.push(Disk { number: 1, partition_style: Some("GPT".into()), partitions: vec![part(1, None, ESP_GUID, MB, 600 * MB, None)], ..Default::default() });
        assert!(plan(&h, &req()).is_ok());
    }
}

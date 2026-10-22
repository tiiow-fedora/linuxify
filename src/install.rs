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

use crate::chooser::{Installer, SecureBootSupport};
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
/// Memory the live desktop and installer need on top of a `toram` copy of the ISO.
/// Xubuntu 26.04 (5.2 GB ISO) copied into 6 GB of RAM hung before its desktop
/// appeared (VM test); Mint 22.3 (2.9 GB) in 6 GB was fine.
pub const TORAM_HEADROOM: u64 = 3 * GB;

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
    /// Ask the live system to copy itself into RAM (frees LNXINST for the installer).
    /// Only for Ubiquity, the one installer that must unmount its live medium:
    /// Calamares breaks with it (Lubuntu: unpackfs can't loop-mount the RAM copy)
    /// and the others install fine with LNXINST still mounted.
    #[serde(default)]
    pub copy_to_ram: bool,
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
    pub installer: Installer,
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
            "this ISO contains a {:.2} GB file, and FAT32 (which the installer partition must use so the PC can boot it) can't hold files over 4 GB. Use a USB stick for this distro.",
            req.largest_iso_file as f64 / GB as f64
        ));
    }
    if req.installer == Installer::Ubiquity {
        let need = req.iso_bytes.saturating_add(TORAM_HEADROOM);
        let ram = hw.ram_bytes.unwrap_or(0);
        if ram < need {
            why.push(format!(
                "this distro's installer has to run from memory when it starts from the disk it installs to. That needs {:.1} GB of RAM and this PC has {:.1} GB. Use a USB stick for this distro.",
                need as f64 / GB as f64,
                ram as f64 / GB as f64
            ));
        }
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
    // diskpart starts new partitions on a 1 MiB boundary. C:'s end isn't
    // always aligned (e.g. after an earlier "extend"), so plan the same
    // rounding diskpart will do. The few KB skipped come out of Linux's share.
    let installer_offset = round_up(part.offset + c_after, MB);
    Ok(InstallPlan {
        distro_id: req.distro_id.clone(),
        iso: req.iso.clone(),
        disk: disk.number,
        c_partition: part.number,
        c_offset: part.offset,
        c_size_before: part.size,
        shrink,
        c_size_after: c_after,
        installer_offset,
        installer_size: installer,
        linux_size: linux,
        letter: req.free_letter,
        copy_to_ram: req.installer == Installer::Ubiquity,
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
    /// Secure Boot is on in the firmware right now.
    pub secure_boot_on: bool,
    /// Whether the chosen distro's installer starts with Secure Boot on (data/distros.json).
    pub distro_secure_boot: SecureBootSupport,
    pub distro_name: String,
    /// A laptop running on battery could power off halfway through shrinking C:.
    pub on_battery: bool,
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
    // Checked before any disk change: firmware with Secure Boot on refuses to start an
    // unsigned installer, which would leave C: shrunk and nothing to install from.
    if p.secure_boot_on {
        match p.distro_secure_boot {
            SecureBootSupport::NeedsDisabling => v.push(format!(
                "{}'s installer can't start while Secure Boot is on. Turn Secure Boot off in your PC's firmware settings first, or choose a distro that supports it",
                p.distro_name
            )),
            SecureBootSupport::Unknown => v.push(format!(
                "Secure Boot is on and Linuxify doesn't know whether {}'s installer starts with it on. Turn Secure Boot off first, or choose a distro that supports it",
                p.distro_name
            )),
            SecureBootSupport::Supported | SecureBootSupport::NeedsKeyEnrolment => {}
        }
    }
    if p.on_battery {
        v.push("this PC is running on battery; plug in the charger so it can't power off while C: is being shrunk".into());
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

/// What to pick in the distro's installer so Linux goes into the free space
/// Linuxify left, and Windows is not shrunk again or erased. Each wording was
/// checked against the real installer in the Windows VM test.
pub fn installer_hint(installer: Installer) -> &'static str {
    match installer {
        Installer::Ubiquity | Installer::UbuntuDesktop => {
            "In the installer choose 'Install … alongside Windows Boot Manager'."
        }
        Installer::Calamares => {
            "In the installer choose 'Replace a partition', then click the grey 'Free Space' block. Don't pick 'Install alongside' (it shrinks Windows again) or 'Erase disk'."
        }
        Installer::Anaconda | Installer::Unknown => {
            "In the installer, install into the free (unallocated) space. Never pick 'Erase disk' or anything that deletes or shrinks the Windows partition."
        }
    }
}

/// Boot config files on the installer partition that `adapt_live_boot` rewrites.
pub const LIVE_BOOT_CONFIGS: [&str; 4] = [r"boot\grub\grub.cfg", r"boot\grub\loopback.cfg", r"boot\grub2\grub.cfg", r"EFI\BOOT\grub.cfg"];

/// Live systems are built to boot from a USB stick or DVD. Booted from a
/// partition on the same disk Linux will be installed to, several of them
/// misbehave (all found in the Windows VM test). This rewrites the kernel
/// lines in an ISO's GRUB config so they run cleanly from LNXINST:
///
/// - Ubuntu family (casper, `/casper/vmlinuz`): casper creates a "writable"
///   persistence partition in any free space on its boot disk, swallowing the
///   space meant for Linux, so `nopersistent`. It also keeps LNXINST mounted
///   at /cdrom, which stops Ubiquity (Mint) repartitioning the disk, so `toram`
///   when `to_ram` is set. casper's own memory check only asks whether the copy
///   fits, not whether the desktop still fits beside it, so the caller decides.
/// - Debian live-boot (`boot=live`): `toram` (when `to_ram`), for the same /cdrom reason.
/// - dracut live images (Fedora, AlmaLinux, openSUSE: `root=live:`) and
///   Anaconda (`inst.stage2=`) find their files by the ISO's volume label,
///   which LNXINST doesn't have, so the label is replaced by `label`.
///   openSUSE turns on a persistent overlay (`rd.live.overlay.persistent`),
///   which is removed for the same reason as casper's.
/// - GRUB's own `search ... -l '<label>'` gets the new label too.
///
/// Returns the patched text and how many lines were changed. Idempotent.
pub fn adapt_live_boot(cfg: &str, label: &str, to_ram: bool) -> (String, usize) {
    let mut changed = 0;
    let out: Vec<String> = cfg
        .split_inclusive('\n')
        .map(|line| {
            let new = adapt_line(line, label, to_ram);
            if new != line {
                changed += 1;
            }
            new
        })
        .collect();
    (out.concat(), changed)
}

fn adapt_line(line: &str, label: &str, to_ram: bool) -> String {
    let body = line.trim_end_matches(['\r', '\n']);
    let ending = &line[body.len()..];
    let indent = &body[..body.len() - body.trim_start().len()];
    let words: Vec<&str> = body.split_whitespace().collect();
    let Some(&cmd) = words.first() else { return line.to_string() };

    if cmd == "search" {
        // search --no-floppy --set=root -l 'Fedora-WS-Live-44'  (also --label)
        let Some(i) = words.iter().position(|w| *w == "-l" || *w == "--label") else { return line.to_string() };
        let Some(old) = words.get(i + 1) else { return line.to_string() };
        if old.trim_matches(['\'', '"']) == label {
            return line.to_string();
        }
        return format!("{indent}{}{ending}", body.trim_start().replacen(old, &format!("'{label}'"), 1));
    }
    if !(cmd == "linux" || cmd == "linuxefi") || words.len() < 2 {
        return line.to_string();
    }

    let kernel = words[1];
    let mut args: Vec<String> = words[2..].iter().map(|w| w.to_string()).collect();
    let has = |args: &[String], f: &str| args.iter().any(|a| a == f);
    let mut add: Vec<&str> = Vec::new();
    if kernel.contains("/casper/") || has(&args, "boot=casper") {
        for f in ["nopersistent", "toram"] {
            if !has(&args, f) && (f != "toram" || to_ram) {
                add.push(f);
            }
        }
    }
    if to_ram && has(&args, "boot=live") && !has(&args, "toram") {
        add.push("toram");
    }
    let before = args.clone();
    args.retain(|a| a != "rd.live.overlay.persistent" && !a.starts_with("rd.live.overlay.cowfs="));
    for a in args.iter_mut() {
        for key in ["root=live:CDLABEL=", "root=live:LABEL=", "inst.stage2=hd:LABEL=", "inst.repo=hd:LABEL="] {
            if a.starts_with(key) {
                *a = format!("{}{label}", key.replace("CDLABEL=", "LABEL="));
            }
        }
    }
    if add.is_empty() && args == before {
        return line.to_string();
    }
    // New flags go right after the kernel path, so they stay before casper's `---`
    // (anything after it is also copied into the installed system's boot line).
    let mut all: Vec<String> = vec![cmd.to_string(), kernel.to_string()];
    all.extend(add.iter().map(|a| a.to_string()));
    all.extend(args);
    format!("{indent}{}{ending}", all.join(" "))
}

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
/// Allows up to 1 MiB of alignment slack on the start, and 2 MB on the size.
pub fn find_partition_at(hw: &HardwareFacts, disk: u32, offset: u64, size: u64) -> Option<u32> {
    hw.disks
        .iter()
        .find(|d| d.number == disk)?
        .partitions
        .iter()
        .find(|p| p.offset.abs_diff(offset) < MB && p.size.abs_diff(size) < 2 * MB)
        .map(|p| p.number)
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
            on_battery: false,
            probe_errors: BTreeMap::new(),
        }
    }

    fn req() -> InstallRequest {
        InstallRequest { distro_id: "linux-mint".into(), iso: "mint.iso".into(), iso_bytes: 3_091_660_800, largest_iso_file: 2_900_000_000, installer: Installer::Ubiquity, linux_gb: None, free_letter: 'L' }
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
        // Installer starts where the shrunk C: ends (aligned here already); Linux space follows it.
        assert_eq!(p.installer_offset, p.c_offset + p.c_size_after);
        assert_eq!(p.installer_offset % MB, 0);
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
    fn unaligned_c_gets_an_aligned_installer_offset() {
        // Seen in the Windows VM: after diskpart "extend", C: was 136,884,239,872
        // bytes (not a whole MiB) and diskpart moved the new partition 16.5 KB on.
        let mut h = hw();
        h.disks[0].partitions[2].offset = 554_696_704;
        h.disks[0].partitions[2].size = 136_884_239_872;
        h.disks[0].partitions[2].free = Some(110 * GB);
        h.c_shrinkable = Some(100 * GB);
        let mut r = req();
        r.linux_gb = Some(85); // what the VM's plan chose: shrink 89 GB
        let p = plan(&h, &r).unwrap();
        assert_eq!(p.shrink, 89 * GB);
        assert_eq!(p.installer_offset % MB, 0);
        assert_eq!(p.installer_offset, 41_875_931_136, "must match where diskpart really put it");
        assert!(p.installer_offset >= p.c_offset + p.c_size_after);
        assert!(p.installer_offset + p.installer_size <= p.c_offset + p.c_size_before);
        // And the partition diskpart made there is recognised.
        h.disks[0].partitions.push(part(5, Some('L'), BASIC, 41_875_931_136, 4 * GB, None));
        assert_eq!(find_partition_at(&h, 0, p.installer_offset, p.installer_size), Some(5));
    }

    #[test]
    fn installer_sizing() {
        assert_eq!(installer_size_for(1_500_000_000), 4 * GB);
        assert_eq!(installer_size_for(3_091_660_800), 4 * GB);
        assert_eq!(installer_size_for(6 * GB), 8 * GB);
    }

    #[test]
    fn preflight_requires_everything() {
        let ok = Preflight { is_admin: true, iso_verified: true, backup_verified: Some(true), distro_secure_boot: SecureBootSupport::Supported, ..Default::default() };
        assert!(preflight_problems(&ok).is_empty());
        let bad = Preflight { is_admin: false, blockers: vec!["BitLocker is on for C:".into()], iso_verified: false, backup_verified: None, backup_on_windows_disk: true, already_installed_state: true, ..ok.clone() };
        let p = preflight_problems(&bad);
        assert_eq!(p.len(), 6, "{p:?}");
        let failed_backup = Preflight { backup_verified: Some(false), ..ok.clone() };
        assert!(preflight_problems(&failed_backup)[0].contains("failed verification"));
        let same_disk = Preflight { backup_on_windows_disk: true, ..ok.clone() };
        assert!(preflight_problems(&same_disk)[0].contains("same disk"));
    }

    #[test]
    fn secure_boot_gate_matches_the_distro() {
        let base = Preflight { is_admin: true, iso_verified: true, backup_verified: Some(true), secure_boot_on: true, distro_name: "Arch Linux".into(), ..Default::default() };
        let with = |s| preflight_problems(&Preflight { distro_secure_boot: s, ..base.clone() });
        assert!(with(SecureBootSupport::Supported).is_empty());
        assert!(with(SecureBootSupport::NeedsKeyEnrolment).is_empty());
        let p = with(SecureBootSupport::NeedsDisabling);
        assert_eq!(p.len(), 1);
        assert!(p[0].contains("Arch Linux") && p[0].contains("Turn Secure Boot off"));
        assert_eq!(with(SecureBootSupport::Unknown).len(), 1);
        // Secure Boot off: any distro is fine.
        let off = Preflight { secure_boot_on: false, distro_secure_boot: SecureBootSupport::NeedsDisabling, ..base };
        assert!(preflight_problems(&off).is_empty());
    }

    #[test]
    fn battery_power_blocks_the_install() {
        let ok = Preflight { is_admin: true, iso_verified: true, backup_verified: Some(true), ..Default::default() };
        assert!(preflight_problems(&ok).is_empty());
        let p = preflight_problems(&Preflight { on_battery: true, ..ok });
        assert_eq!(p.len(), 1);
        assert!(p[0].contains("charger"));
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
    fn live_boot_is_adapted_for_every_family() {
        // Real lines from the offered ISOs' boot configs (2026-10).
        let mint = "menuentry \"Start Linux Mint\" {\n\tlinux\t/casper/vmlinuz  boot=casper uuid=6e72 username=mint quiet splash --\n\tinitrd\t/casper/initrd.lz\n}\n\tlinux\t/boot/memtest.efi\n";
        let (out, n) = adapt_live_boot(mint, "LNXINST", true);
        assert_eq!(n, 1);
        assert!(out.contains("\tlinux /casper/vmlinuz nopersistent toram boot=casper uuid=6e72 username=mint quiet splash --\n"), "{out}");
        assert!(out.contains("\tlinux\t/boot/memtest.efi\n"), "non-live entries untouched");

        // Ubuntu 26.04 family: no boot=casper any more; flags must stay before `---`.
        let ubuntu = "  linux  /casper/vmlinuz  --- quiet splash\r\n";
        assert_eq!(adapt_live_boot(ubuntu, "LNXINST", true).0, "  linux /casper/vmlinuz nopersistent toram --- quiet splash\r\n");

        let debian = "linux\t/live/vmlinuz-6.12 boot=live components quiet splash findiso=${iso_path}\n";
        assert_eq!(adapt_live_boot(debian, "LNXINST", true).0, "linux /live/vmlinuz-6.12 toram boot=live components quiet splash findiso=${iso_path}\n");
        // Not enough RAM for a copy: casper still gets nopersistent, nobody gets toram.
        assert_eq!(adapt_live_boot(ubuntu, "LNXINST", false).0, "  linux /casper/vmlinuz nopersistent --- quiet splash\r\n");
        assert_eq!(adapt_live_boot(debian, "LNXINST", false), (debian.to_string(), 0));

        let fedora = "linux ($root)/boot/x86_64/loader/linux quiet rhgb  root=live:CDLABEL=Fedora-WS-Live-44 rd.live.image\n";
        assert_eq!(adapt_live_boot(fedora, "LNXINST", true).0, "linux ($root)/boot/x86_64/loader/linux quiet rhgb root=live:LABEL=LNXINST rd.live.image\n");

        let alma = "  search --no-floppy --set=root -l 'AlmaLinux-10_2-x86_64-GNOME'\n  linuxefi /images/pxeboot/vmlinuz root=live:CDLABEL=AlmaLinux-10_2-x86_64-GNOME  rd.live.image quiet rhgb\n";
        let (a, n) = adapt_live_boot(alma, "LNXINST", true);
        assert_eq!(n, 2);
        assert_eq!(a, "  search --no-floppy --set=root -l 'LNXINST'\n  linuxefi /images/pxeboot/vmlinuz root=live:LABEL=LNXINST rd.live.image quiet rhgb\n");

        let bazzite = "linuxefi /images/pxeboot/vmlinuz inst.stage2=hd:LABEL=bazzite-x86_64-stable quiet\n";
        assert_eq!(adapt_live_boot(bazzite, "LNXINST", true).0, "linuxefi /images/pxeboot/vmlinuz inst.stage2=hd:LABEL=LNXINST quiet\n");

        let suse = "linux ($root)/boot/x86_64/loader/linux ${extra_cmdline} quiet root=live:CDLABEL=openSUSE_Tumbleweed_KDE_Live rd.live.image rd.live.overlay.persistent rd.live.overlay.cowfs=ext4\n";
        assert_eq!(adapt_live_boot(suse, "LNXINST", true).0, "linux ($root)/boot/x86_64/loader/linux ${extra_cmdline} quiet root=live:LABEL=LNXINST rd.live.image\n");

        // Idempotent, and leaves others alone (Arch's loopback, NixOS).
        for cfg in [mint, ubuntu, debian, fedora, alma, bazzite, suse] {
            let once = adapt_live_boot(cfg, "LNXINST", true).0;
            assert_eq!(adapt_live_boot(&once, "LNXINST", true), (once.clone(), 0));
        }
        let arch = "linux /arch/boot/x86_64/vmlinuz-linux archisobasedir=arch img_dev=UUID=${archiso_img_dev_uuid}\n";
        assert_eq!(adapt_live_boot(arch, "LNXINST", true), (arch.to_string(), 0));
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
        assert_eq!(find_partition_at(&h, 0, p.installer_offset + 2 * MB, p.installer_size), None);
    }

    #[test]
    fn plan_on_this_machine_layout_esp_on_other_disk() {
        // Developer PC: C: alone on disk 0 (MSR + C:), ESP on disk 1.
        let mut h = hw();
        h.disks[0].partitions.retain(|p| p.number != 1);
        h.disks.push(Disk { number: 1, partition_style: Some("GPT".into()), partitions: vec![part(1, None, ESP_GUID, MB, 600 * MB, None)], ..Default::default() });
        assert!(plan(&h, &req()).is_ok());
    }

    #[test]
    fn copy_to_ram_only_for_ubiquity_with_headroom() {
        let mut h = hw();
        h.ram_bytes = Some(6 * GB); // Mint in the 6 GB VM: fine with toram
        assert!(plan(&h, &req()).unwrap().copy_to_ram);
        h.ram_bytes = Some(4 * GB);
        let why = plan(&h, &req()).unwrap_err();
        assert!(why.iter().any(|w| w.contains("RAM") && w.contains("USB stick")), "{why:?}");
        // Calamares (Lubuntu) broke with toram; Ubuntu's installer doesn't need it.
        for i in [Installer::Calamares, Installer::UbuntuDesktop, Installer::Anaconda, Installer::Unknown] {
            let p = plan(&h, &InstallRequest { installer: i, ..req() }).unwrap();
            assert!(!p.copy_to_ram, "{i:?}");
        }
    }

    #[test]
    fn installer_hints_steer_away_from_shrinking_windows() {
        assert!(installer_hint(Installer::Ubiquity).contains("alongside Windows Boot Manager"));
        let c = installer_hint(Installer::Calamares);
        assert!(c.contains("Replace a partition") && c.contains("Free Space") && c.contains("Don't pick 'Install alongside'"));
        assert!(installer_hint(Installer::Unknown).contains("Never pick 'Erase disk'"));
    }
}

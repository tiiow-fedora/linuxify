//! Turns scan facts into readiness findings. Blockers here are the hard
//! safety rule #4 conditions: later stages must refuse to continue while any
//! blocker is present. Pure logic, fully unit-tested.

use crate::mapping::{Group, MappedProgram, Status};
use crate::scan::hardware::{BitLocker, Firmware, HardwareFacts, SecureBoot, Vendor};
use crate::scan::userdata::UserData;
use serde::{Deserialize, Serialize};

pub const MIN_SHRINK_BYTES: u64 = 50 * 1_000_000_000;
/// ESP GPT partition type GUID.
pub const ESP_GUID: &str = "{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Blocker,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Stable machine id, e.g. "bitlocker-active". Later stages check these.
    pub id: String,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub how_to_fix: Option<String>,
}

fn f(id: &str, severity: Severity, title: &str, detail: String, fix: Option<&str>) -> Finding {
    Finding { id: id.into(), severity, title: title.into(), detail, how_to_fix: fix.map(Into::into) }
}

pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

pub fn assess(hw: &HardwareFacts, hw_error: Option<&str>, programs: &[MappedProgram], data: &UserData) -> Vec<Finding> {
    let mut out = Vec::new();

    if let Some(e) = hw_error {
        out.push(f(
            "hardware-probe-failed",
            Severity::Blocker,
            "Hardware check failed",
            format!("Linuxify could not read disk/boot information, so it cannot tell whether installing is safe. Error: {e}"),
            Some("Re-run `linuxify scan`. If it keeps failing, run it from an administrator terminal."),
        ));
    }

    // --- BitLocker on the Windows volume -----------------------------------
    match hw.bitlocker.iter().find(|b| b.mount.eq_ignore_ascii_case("C:")) {
        Some(b) if b.state == BitLocker::Active => out.push(f(
            "bitlocker-active",
            Severity::Blocker,
            "BitLocker is on for C:",
            format!("C: is encrypted ({}). Shrinking it or changing boot settings can trigger the BitLocker recovery screen, and Linux cannot read an encrypted C:.", b.detail),
            Some("1) Save your recovery key (Settings → Privacy & security → Device encryption, or https://aka.ms/myrecoverykey). 2) Either turn BitLocker off (Control Panel → BitLocker Drive Encryption → Turn off; wait for decryption to finish) or, for a dual boot, suspend it right before installing (admin terminal: manage-bde -protectors -disable C: -RebootCount 3). Then re-run the scan."),
        )),
        Some(b) if b.state == BitLocker::Off => {}
        _ => out.push(f(
            "bitlocker-unknown",
            Severity::Blocker,
            "Could not tell whether C: is encrypted (BitLocker)",
            "The BitLocker check gave no answer, so it is not safe to assume the drive is unencrypted.".into(),
            Some("Run `linuxify scan` from an administrator terminal, or check Control Panel → BitLocker Drive Encryption."),
        )),
    }

    // --- Intel RST / VMD ----------------------------------------------------
    if hw.rst_vmd.detected {
        out.push(f(
            "rst-vmd",
            Severity::Blocker,
            "Disk controller is in Intel RST/VMD (RAID) mode",
            format!("Most Linux installers cannot see disks behind Intel RST/VMD. Evidence: {}", hw.rst_vmd.evidence.join("; ")),
            Some("Switch the SATA/NVMe mode to AHCI in the BIOS. Do it safely so Windows still boots: 1) admin terminal: bcdedit /set {current} safeboot minimal 2) reboot into BIOS, set AHCI (or disable VMD), save 3) Windows starts in Safe Mode 4) admin terminal: bcdedit /deletevalue {current} safeboot 5) reboot. Back up first."),
        ));
    }

    // --- Space to shrink C: -------------------------------------------------
    let other_space: Vec<String> = hw
        .disks
        .iter()
        .filter(|d| d.unallocated >= MIN_SHRINK_BYTES)
        .map(|d| format!("disk {} ({}) has {} unallocated", d.number, d.model.as_deref().unwrap_or("?"), gb(d.unallocated)))
        .collect();
    let alt = if other_space.is_empty() { String::new() } else { format!(" Alternatively, {}.", other_space.join("; ")) };
    match (hw.c_shrinkable, hw.c_drive.as_ref()) {
        (Some(s), _) if s < MIN_SHRINK_BYTES => out.push(f(
            "low-space",
            Severity::Blocker,
            "Not enough room to shrink C:",
            format!("Windows can only shrink C: by {} but Linux needs at least {}.{alt}", gb(s), gb(MIN_SHRINK_BYTES)),
            Some("Free up space (Disk Cleanup, uninstall unused programs/games, move files to another drive), disable hibernation (admin: powercfg /h off) and re-run the scan."),
        )),
        (None, Some(c)) if c.free < MIN_SHRINK_BYTES => out.push(f(
            "low-space",
            Severity::Blocker,
            "Not enough free space on C:",
            format!("C: has {} free; Linux needs at least {}.{alt}", gb(c.free), gb(MIN_SHRINK_BYTES)),
            Some("Free up space (Disk Cleanup, uninstall unused programs/games, move files to another drive) and re-run the scan."),
        )),
        (None, Some(c)) => out.push(f(
            "shrink-unverified",
            Severity::Info,
            "Free space looks fine (exact shrink size not checked)",
            format!("C: has {} free. Windows sometimes can't shrink all of it because of unmovable system files; the exact figure needs admin rights.", gb(c.free)),
            Some("Optional: run `linuxify scan` from an administrator terminal to get the exact shrinkable size."),
        )),
        (None, None) => out.push(f(
            "space-unknown",
            Severity::Blocker,
            "Could not read free space on C:",
            "Without this, Linuxify can't tell whether there is room for Linux.".into(),
            Some("Re-run the scan; if it persists, run it as administrator."),
        )),
        _ => {}
    }

    // --- Firmware and Secure Boot ------------------------------------------
    match hw.firmware {
        Firmware::Bios => out.push(f(
            "legacy-bios",
            Severity::Warning,
            "Windows boots in legacy BIOS mode",
            "Linux works in BIOS mode, but it must be installed in the same mode as Windows for dual boot to work. Most guides assume UEFI.".into(),
            Some("Nothing to do now; Linuxify will set up the installer in BIOS mode. (Converting Windows to UEFI with mbr2gpt is possible but is a separate, risky job.)"),
        )),
        Firmware::Unknown => out.push(f(
            "firmware-unknown",
            Severity::Blocker,
            "Could not tell whether this PC uses UEFI or BIOS",
            "Boot setup depends on this.".into(),
            Some("Run msinfo32 and look at 'BIOS Mode', or re-run the scan as administrator."),
        )),
        Firmware::Uefi => {}
    }
    match hw.secure_boot {
        SecureBoot::Unknown => out.push(f(
            "secure-boot-unknown",
            Severity::Blocker,
            "Secure Boot state is unknown",
            "Linuxify needs to know this to pick a boot method that will actually start.".into(),
            Some("Run msinfo32 and look at 'Secure Boot State', or re-run the scan as administrator."),
        )),
        SecureBoot::On => out.push(f(
            "secure-boot-on",
            Severity::Info,
            "Secure Boot is on",
            "Fine for Ubuntu, Linux Mint, Fedora, openSUSE and Pop!_OS. Some distros (and NVIDIA's driver) need an extra one-time key enrolment step or Secure Boot turned off.".into(),
            None,
        )),
        SecureBoot::Off => out.push(f("secure-boot-off", Severity::Info, "Secure Boot is off", "Any distro will boot. (Windows 11 normally expects it on; that's your choice.)".into(), None)),
        SecureBoot::NotSupported => {}
    }

    // --- Where the EFI partition is ----------------------------------------
    let esps: Vec<(u32, String, u64)> = hw
        .disks
        .iter()
        .flat_map(|d| d.partitions.iter().filter(|p| p.gpt_type.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(ESP_GUID))).map(move |p| (d.number, d.model.clone().unwrap_or_default(), p.size)))
        .collect();
    let windows_disk = hw.disks.iter().find(|d| d.partitions.iter().any(|p| p.drive_letter == Some('C')));
    if let (Some(wd), Some((esp_disk, esp_model, _))) = (windows_disk, esps.first()) {
        if *esp_disk != wd.number {
            out.push(f(
                "esp-other-disk",
                Severity::Info,
                "Boot partition is on a different disk from Windows",
                format!("Windows (C:) is on disk {} but the EFI boot partition is on disk {} ({esp_model}). The installer must use that same boot partition; unplugging disk {esp_disk} would stop both systems booting.", wd.number, esp_disk),
                None,
            ));
        }
    }
    for (disk, _, size) in &esps {
        if *size < 300_000_000 {
            out.push(f(
                "esp-small",
                Severity::Warning,
                "EFI boot partition is small",
                format!("The EFI partition on disk {disk} is {}. Some distros (e.g. Pop!_OS) want 512 MB or more; Ubuntu, Mint and Fedora usually manage with 100–260 MB.", gb(*size)),
                None,
            ));
        }
    }

    // --- Fast Startup -------------------------------------------------------
    if hw.fast_startup == Some(true) {
        out.push(f(
            "fast-startup",
            Severity::Warning,
            "Windows Fast Startup is on",
            "Fast Startup leaves the Windows drive half-hibernated, so Linux can't safely write to it and may refuse to mount it.".into(),
            Some("Control Panel → Power Options → Choose what the power buttons do → untick 'Turn on fast startup'. (Or admin: powercfg /h off.)"),
        ));
    }

    if hw.on_battery {
        out.push(f(
            "on-battery",
            Severity::Warning,
            "Running on battery",
            "Shrinking C: and setting up the installer must not be interrupted by the PC powering off.".into(),
            Some("Plug in the charger before `linuxify install` (it refuses to run on battery)."),
        ));
    }

    // --- Hardware compatibility ---------------------------------------------
    if hw.os.as_ref().and_then(|o| o.arch.as_deref()).is_some_and(|a| a.to_lowercase().contains("arm")) {
        out.push(f("arm-cpu", Severity::Blocker, "This is an ARM PC", "Linux support for Windows-on-ARM laptops is still limited and most distros' installers won't boot on them.".into(), None));
    }
    let real_gpus: Vec<_> = hw.gpus.iter().filter(|g| !g.is_virtual).collect();
    if real_gpus.iter().any(|g| g.vendor == Vendor::Nvidia) {
        out.push(f(
            "nvidia-gpu",
            Severity::Warning,
            "NVIDIA graphics card",
            "NVIDIA cards need NVIDIA's own driver for good performance and gaming. Distros like Pop!_OS, Bazzite, Linux Mint and Ubuntu make this a checkbox; on others it's a manual step.".into(),
            None,
        ));
    }
    for w in &hw.wifi {
        if matches!(w.chipset_vendor.as_str(), "Broadcom" | "Realtek" | "Unknown") {
            out.push(f("wifi-driver", Severity::Warning, "Wi-Fi may need an extra driver", format!("{}: {}", w.description, w.linux_note), None));
        }
    }
    if let Some(ram) = hw.ram_bytes {
        if ram < 4 * 1024 * 1024 * 1024 {
            out.push(f("low-ram", Severity::Warning, "Less than 4 GB of RAM", format!("{} of RAM. A lightweight desktop (Xfce, LXQt) will feel much better.", gb(ram)), None));
        }
    }

    // --- Programs that won't come along ------------------------------------
    let blocked: Vec<&str> = programs
        .iter()
        .filter(|p| p.group == Group::Mapped && p.status == Status::Blocked)
        .map(|p| p.program.name.as_str())
        .collect();
    if !blocked.is_empty() {
        out.push(f(
            "blocked-apps",
            Severity::Warning,
            "Some of your programs won't work on Linux",
            format!("{} program(s) are known not to run on Linux: {}. Keeping Windows as a dual boot lets you still use them.", blocked.len(), blocked.join(", ")),
            None,
        ));
    }

    // --- Data that needs attention before migrating ------------------------
    let cloud_only: u64 = data.folders.iter().map(|d| d.stats.cloud_only_bytes).sum::<u64>();
    if cloud_only > 0 {
        out.push(f(
            "cloud-only-files",
            Severity::Warning,
            "Some files are online-only (OneDrive)",
            format!("{} of files in your folders exist only in the cloud. They must be downloaded before Linuxify can copy them.", gb(cloud_only)),
            Some("In File Explorer, right-click the folder → 'Always keep on this device', and wait for the download to finish."),
        ));
    }

    if !hw.is_admin {
        out.push(f(
            "not-admin",
            Severity::Info,
            "Scan ran without administrator rights",
            "Everything important was checked, but a few details (exact shrinkable space, BitLocker detail) are more precise as administrator.".into(),
            None,
        ));
    }

    out.sort_by_key(|x| x.severity);
    out
}

pub fn has_blockers(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == Severity::Blocker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::hardware::*;
    use crate::scan::userdata::UserData;
    use std::collections::BTreeMap;

    fn healthy() -> HardwareFacts {
        HardwareFacts {
            is_admin: true,
            os: None,
            cpus: vec![],
            ram_bytes: Some(16 << 30),
            computer: None,
            gpus: vec![],
            disks: vec![],
            storage_controllers: vec![],
            wifi: vec![],
            firmware: Firmware::Uefi,
            secure_boot: SecureBoot::On,
            bitlocker: vec![BitLockerVolume { mount: "C:".into(), state: BitLocker::Off, detail: "Off".into() }],
            rst_vmd: RstVmd { detected: false, evidence: vec![] },
            c_drive: Some(CDrive { size: 1_000_000_000_000, free: 400_000_000_000, file_system: Some("NTFS".into()) }),
            c_shrinkable: Some(300_000_000_000),
            fast_startup: Some(false),
            on_battery: false,
            probe_errors: BTreeMap::new(),
        }
    }

    fn ids(hw: &HardwareFacts) -> Vec<String> {
        assess(hw, None, &[], &UserData::default()).into_iter().map(|f| f.id).collect()
    }

    fn blockers(hw: &HardwareFacts) -> Vec<String> {
        assess(hw, None, &[], &UserData::default()).into_iter().filter(|f| f.severity == Severity::Blocker).map(|f| f.id).collect()
    }

    #[test]
    fn healthy_machine_has_no_blockers() {
        assert!(blockers(&healthy()).is_empty(), "{:?}", ids(&healthy()));
    }

    #[test]
    fn bitlocker_active_or_unknown_blocks() {
        let mut hw = healthy();
        hw.bitlocker[0].state = BitLocker::Active;
        assert_eq!(blockers(&hw), vec!["bitlocker-active"]);
        hw.bitlocker[0].state = BitLocker::Unknown;
        assert_eq!(blockers(&hw), vec!["bitlocker-unknown"]);
        hw.bitlocker.clear();
        assert_eq!(blockers(&hw), vec!["bitlocker-unknown"]);
    }

    #[test]
    fn rst_blocks() {
        let mut hw = healthy();
        hw.rst_vmd = RstVmd { detected: true, evidence: vec!["x".into()] };
        assert_eq!(blockers(&hw), vec!["rst-vmd"]);
    }

    #[test]
    fn low_space_blocks_using_shrinkable_when_known() {
        let mut hw = healthy();
        hw.c_shrinkable = Some(20_000_000_000); // lots free, but can't shrink
        assert_eq!(blockers(&hw), vec!["low-space"]);
        hw.c_shrinkable = None;
        assert!(ids(&hw).contains(&"shrink-unverified".to_string()));
        assert!(blockers(&hw).is_empty());
        hw.c_drive.as_mut().unwrap().free = 10_000_000_000;
        assert_eq!(blockers(&hw), vec!["low-space"]);
        hw.c_drive = None;
        assert_eq!(blockers(&hw), vec!["space-unknown"]);
    }

    #[test]
    fn secure_boot_unknown_blocks() {
        let mut hw = healthy();
        hw.secure_boot = SecureBoot::Unknown;
        assert_eq!(blockers(&hw), vec!["secure-boot-unknown"]);
    }

    #[test]
    fn probe_failure_blocks() {
        let hw = healthy();
        let b: Vec<_> = assess(&hw, Some("boom"), &[], &UserData::default()).into_iter().filter(|f| f.severity == Severity::Blocker).map(|f| f.id).collect();
        assert_eq!(b, vec!["hardware-probe-failed"]);
    }

    #[test]
    fn warnings_for_fast_startup_and_nvidia() {
        let mut hw = healthy();
        hw.fast_startup = Some(true);
        hw.gpus.push(Gpu { name: "RTX".into(), vendor: Vendor::Nvidia, is_virtual: false, driver_version: None });
        let i = ids(&hw);
        assert!(i.contains(&"fast-startup".to_string()));
        assert!(i.contains(&"nvidia-gpu".to_string()));
        assert!(blockers(&hw).is_empty());
    }

    #[test]
    fn esp_on_other_disk_is_noted() {
        let mut hw = healthy();
        let part = |n: u32, letter: Option<char>, gpt: &str, size: u64| Partition { number: n, drive_letter: letter, size, gpt_type: Some(gpt.into()), ..Default::default() };
        hw.disks = vec![
            Disk { number: 0, partitions: vec![part(2, Some('C'), "{ebd0a0a2-b9e5-4433-87c0-68b6b72699c7}", 1 << 40)], ..Default::default() },
            Disk { number: 1, partitions: vec![part(1, None, ESP_GUID, 100_000_000)], ..Default::default() },
        ];
        let i = ids(&hw);
        assert!(i.contains(&"esp-other-disk".to_string()));
        assert!(i.contains(&"esp-small".to_string()));
    }

    #[test]
    fn findings_sorted_blockers_first() {
        let mut hw = healthy();
        hw.fast_startup = Some(true);
        hw.secure_boot = SecureBoot::Unknown;
        let all = assess(&hw, None, &[], &UserData::default());
        assert_eq!(all[0].severity, Severity::Blocker);
    }
}

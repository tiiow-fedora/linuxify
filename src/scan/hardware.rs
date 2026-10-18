//! Hardware and boot facts.
//!
//! Most facts come from `hardware_probe.ps1`, a read-only PowerShell/CIM
//! script that prints one JSON object. A few boot facts come straight from
//! the registry because they are readable without admin rights there.
//! Interpretation (vendors, RST/VMD, BitLocker codes) is pure and tested.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

const PROBE_SCRIPT: &str = include_str!("hardware_probe.ps1");

// ---------------------------------------------------------------------------
// Raw probe JSON
// ---------------------------------------------------------------------------

/// PowerShell 5.1 sometimes emits a single object where we expect a list,
/// and `null` for empty lists. Accept all three shapes.
fn one_or_many<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        Many(Vec<T>),
        One(T),
    }
    Ok(match Option::<OneOrMany<T>>::deserialize(d)? {
        None => Vec::new(),
        Some(OneOrMany::Many(v)) => v,
        Some(OneOrMany::One(t)) => vec![t],
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Probe {
    #[serde(default)]
    pub is_admin: bool,
    pub os: Option<OsInfo>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub cpu: Vec<Cpu>,
    pub ram_bytes: Option<u64>,
    pub computer: Option<ComputerInfo>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub gpus: Vec<RawGpu>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub disks: Vec<Disk>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub storage_controllers: Vec<StorageController>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub net_adapters: Vec<RawNetAdapter>,
    pub c_drive: Option<CDrive>,
    pub c_shrink: Option<CShrink>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub bitlocker_cmdlet: Vec<BitLockerCmdlet>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub bitlocker_shell: Vec<BitLockerShell>,
    #[serde(default)]
    pub errors: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OsInfo {
    pub caption: Option<String>,
    pub version: Option<String>,
    pub build: Option<String>,
    pub arch: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cpu {
    pub name: Option<String>,
    pub manufacturer: Option<String>,
    pub cores: Option<u32>,
    pub threads: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComputerInfo {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawGpu {
    pub name: Option<String>,
    pub pnp_id: Option<String>,
    pub driver_version: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Disk {
    pub number: u32,
    pub model: Option<String>,
    pub size: u64,
    pub bus_type: Option<String>,
    pub partition_style: Option<String>,
    #[serde(default)]
    pub is_boot: bool,
    #[serde(default)]
    pub is_system: bool,
    #[serde(default)]
    pub unallocated: u64,
    #[serde(default, deserialize_with = "one_or_many")]
    pub partitions: Vec<Partition>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Partition {
    pub number: u32,
    #[serde(default, deserialize_with = "drive_letter")]
    pub drive_letter: Option<char>,
    pub size: u64,
    #[serde(default)]
    pub offset: u64,
    pub gpt_type: Option<String>,
    pub mbr_type: Option<u32>,
    #[serde(default)]
    pub is_boot: bool,
    #[serde(default)]
    pub is_system: bool,
    #[serde(default)]
    pub is_hidden: bool,
    pub file_system: Option<String>,
    pub label: Option<String>,
    pub free: Option<u64>,
}

/// Get-Partition reports "no letter" as a NUL character.
fn drive_letter<'de, D: Deserializer<'de>>(d: D) -> Result<Option<char>, D::Error> {
    let s = Option::<String>::deserialize(d)?;
    Ok(s.and_then(|s| s.chars().next()).filter(|c| c.is_ascii_alphabetic()))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageController {
    pub name: Option<String>,
    pub pnp_class: Option<String>,
    pub pnp_id: Option<String>,
    pub service: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawNetAdapter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub media: Option<String>,
    pub pnp_id: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CDrive {
    pub size: u64,
    pub free: u64,
    pub file_system: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CShrink {
    pub current: u64,
    pub min: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BitLockerCmdlet {
    pub mount: Option<String>,
    pub protection: Option<String>,
    pub status: Option<String>,
    pub percent: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BitLockerShell {
    pub mount: Option<String>,
    pub value: Option<i64>,
}

// ---------------------------------------------------------------------------
// Interpreted facts (what the report and readiness checks use)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gpu {
    pub name: String,
    pub vendor: Vendor,
    /// Virtual display adapters (remote desktop, VR mirrors) — ignored for advice.
    pub is_virtual: bool,
    pub driver_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiAdapter {
    pub description: String,
    pub chipset_vendor: String,
    pub linux_note: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Firmware {
    Uefi,
    Bios,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecureBoot {
    On,
    Off,
    /// Legacy BIOS boot: Secure Boot does not exist.
    NotSupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BitLocker {
    /// Encrypted (on, suspended, encrypting, decrypting or locked).
    Active,
    Off,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitLockerVolume {
    pub mount: String,
    pub state: BitLocker,
    /// Human description of the raw code, e.g. "On", "Suspended".
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RstVmd {
    pub detected: bool,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareFacts {
    pub is_admin: bool,
    pub os: Option<OsInfo>,
    pub cpus: Vec<Cpu>,
    pub ram_bytes: Option<u64>,
    pub computer: Option<ComputerInfo>,
    pub gpus: Vec<Gpu>,
    pub disks: Vec<Disk>,
    pub storage_controllers: Vec<StorageController>,
    pub wifi: Vec<WifiAdapter>,
    pub firmware: Firmware,
    pub secure_boot: SecureBoot,
    pub bitlocker: Vec<BitLockerVolume>,
    pub rst_vmd: RstVmd,
    pub c_drive: Option<CDrive>,
    /// Bytes Windows says C: can shrink by (needs admin; None otherwise).
    pub c_shrinkable: Option<u64>,
    pub fast_startup: Option<bool>,
    pub probe_errors: BTreeMap<String, String>,
}

/// Registry-sourced boot facts (all readable without admin).
#[derive(Debug, Clone, Default)]
pub struct RegistryBootFacts {
    /// HKLM\SYSTEM\CurrentControlSet\Control\PEFirmwareType (1 BIOS, 2 UEFI)
    pub pe_firmware_type: Option<u32>,
    /// ...\Control\SecureBoot\State\UEFISecureBootEnabled
    pub secure_boot_enabled: Option<u32>,
    /// ...\Session Manager\Power\HiberbootEnabled
    pub hiberboot_enabled: Option<u32>,
    /// The `firmware_type` environment variable Windows sets for every
    /// process ("UEFI" or "Legacy"). Fallback when PEFirmwareType is absent.
    pub firmware_type_env: Option<String>,
}

pub fn pci_vendor(pnp_id: &str) -> Option<&'static str> {
    let up = pnp_id.to_ascii_uppercase();
    let ven = up
        .split(['\\', '&'])
        .find_map(|p| p.strip_prefix("VEN_").or_else(|| p.strip_prefix("VID_")))?;
    Some(match ven {
        "10DE" => "NVIDIA",
        "1002" | "1022" => "AMD",
        "8086" | "8087" => "Intel",
        "14E4" => "Broadcom",
        "168C" | "0CF3" => "Qualcomm Atheros",
        "17CB" => "Qualcomm",
        "10EC" | "0BDA" => "Realtek",
        "14C3" | "0E8D" => "MediaTek",
        "1814" | "148F" => "Ralink/MediaTek",
        "1B4B" => "Marvell",
        "15B7" => "SanDisk/WD",
        _ => return None,
    })
}

pub fn gpu_from_raw(g: &RawGpu) -> Gpu {
    let name = g.name.clone().unwrap_or_else(|| "Unknown GPU".into());
    let pnp = g.pnp_id.clone().unwrap_or_default();
    let lname = name.to_lowercase();
    let vendor = match pci_vendor(&pnp) {
        Some("NVIDIA") => Vendor::Nvidia,
        Some("AMD") => Vendor::Amd,
        Some("Intel") => Vendor::Intel,
        _ if lname.contains("nvidia") || lname.contains("geforce") => Vendor::Nvidia,
        _ if lname.contains("radeon") || lname.contains("amd") => Vendor::Amd,
        _ if lname.contains("intel") => Vendor::Intel,
        _ => Vendor::Other,
    };
    // Real GPUs sit on the PCI bus; ROOT\ / SWD\ devices are software adapters.
    let is_virtual = !pnp.to_ascii_uppercase().starts_with("PCI\\");
    Gpu { name, vendor, is_virtual, driver_version: g.driver_version.clone() }
}

pub fn wifi_from_raw(adapters: &[RawNetAdapter]) -> Vec<WifiAdapter> {
    adapters
        .iter()
        .filter(|a| {
            let d = a.description.as_deref().unwrap_or("").to_lowercase();
            // NdisPhysicalMedium 9 = Native 802.11
            a.media.as_deref() == Some("9")
                || d.contains("wi-fi")
                || d.contains("wireless")
                || d.contains("802.11")
                || d.contains("wlan")
        })
        .map(|a| {
            let description = a.description.clone().unwrap_or_default();
            let vendor = a
                .pnp_id
                .as_deref()
                .and_then(pci_vendor)
                .unwrap_or("Unknown")
                .to_string();
            let usb = a.pnp_id.as_deref().unwrap_or("").to_ascii_uppercase().starts_with("USB\\");
            let linux_note = match vendor.as_str() {
                "Intel" => "Intel Wi-Fi works out of the box on Linux.".to_string(),
                "Qualcomm Atheros" | "Qualcomm" | "MediaTek" => {
                    "Usually works out of the box on current Linux kernels.".to_string()
                }
                "Broadcom" => "Broadcom Wi-Fi often needs an extra proprietary driver (e.g. broadcom-wl); have a cable or phone tethering ready for first boot.".to_string(),
                "Realtek" if usb => "Realtek USB Wi-Fi: some models need an extra driver. Have a cable or phone tethering ready for first boot.".to_string(),
                "Realtek" => "Realtek PCIe Wi-Fi usually works; a few newer models need a recent kernel.".to_string(),
                _ => "Could not identify the chipset; check it works in the live USB/VM trial.".to_string(),
            };
            WifiAdapter { description, chipset_vendor: vendor, linux_note }
        })
        .collect()
}

pub fn detect_rst_vmd(controllers: &[StorageController], disks: &[Disk]) -> RstVmd {
    const SERVICES: [&str; 5] = ["iastorvd", "iastorac", "iastoravc", "iastora", "iastorv"];
    let mut evidence = Vec::new();
    for c in controllers {
        let name = c.name.as_deref().unwrap_or("");
        let lname = name.to_lowercase();
        let svc = c.service.as_deref().unwrap_or("").to_lowercase();
        let name_hit = ["raid", "rapid storage", "volume management device", " vmd", "rst "]
            .iter()
            .any(|k| lname.contains(k));
        // Microsoft Storage Spaces is a software RAID driver, not Intel RST.
        let storage_spaces = svc == "spaceport";
        if (name_hit && !storage_spaces) || SERVICES.contains(&svc.as_str()) {
            evidence.push(format!("Storage controller: {name} (driver: {svc})"));
        }
    }
    for d in disks {
        if d.bus_type.as_deref().is_some_and(|b| b.eq_ignore_ascii_case("RAID")) {
            evidence.push(format!(
                "Disk {} ({}) is attached through a RAID bus",
                d.number,
                d.model.as_deref().unwrap_or("?")
            ));
        }
    }
    RstVmd { detected: !evidence.is_empty(), evidence }
}

/// Shell property System.Volume.BitLockerProtection codes.
/// 2 = off is the only "safe" value; anything encrypted counts as active.
pub fn bitlocker_from_shell_code(code: Option<i64>) -> (BitLocker, &'static str) {
    match code {
        Some(1) => (BitLocker::Active, "On"),
        Some(2) => (BitLocker::Off, "Off"),
        Some(3) => (BitLocker::Active, "Encrypting"),
        Some(4) => (BitLocker::Active, "Decrypting"),
        Some(5) => (BitLocker::Active, "Suspended (still encrypted)"),
        Some(6) => (BitLocker::Active, "On (locked)"),
        Some(8) => (BitLocker::Active, "Device encryption waiting for activation"),
        Some(_) | None => (BitLocker::Unknown, "Could not determine"),
    }
}

pub fn bitlocker_volumes(probe: &Probe) -> Vec<BitLockerVolume> {
    // Prefer the admin-only cmdlet when it worked; it is authoritative.
    if !probe.bitlocker_cmdlet.is_empty() {
        return probe
            .bitlocker_cmdlet
            .iter()
            .map(|b| {
                let status = b.status.clone().unwrap_or_default();
                let protection = b.protection.clone().unwrap_or_default();
                let state = if status == "FullyDecrypted" {
                    BitLocker::Off
                } else if status.is_empty() {
                    BitLocker::Unknown
                } else {
                    BitLocker::Active
                };
                BitLockerVolume {
                    mount: b.mount.clone().unwrap_or_default(),
                    state,
                    detail: format!("{status}, protection {protection}"),
                }
            })
            .collect();
    }
    probe
        .bitlocker_shell
        .iter()
        .map(|b| {
            let (state, detail) = bitlocker_from_shell_code(b.value);
            BitLockerVolume { mount: b.mount.clone().unwrap_or_default(), state, detail: detail.into() }
        })
        .collect()
}

pub fn firmware_from(reg: &RegistryBootFacts) -> Firmware {
    match (reg.pe_firmware_type, reg.firmware_type_env.as_deref().map(str::to_ascii_lowercase).as_deref()) {
        (Some(1), _) => Firmware::Bios,
        (Some(2), _) => Firmware::Uefi,
        (_, Some("uefi")) => Firmware::Uefi,
        (_, Some("legacy")) | (_, Some("bios")) => Firmware::Bios,
        _ => Firmware::Unknown,
    }
}

pub fn secure_boot_from(reg: &RegistryBootFacts, firmware: Firmware) -> SecureBoot {
    match (reg.secure_boot_enabled, firmware) {
        (_, Firmware::Bios) => SecureBoot::NotSupported,
        (Some(1), _) => SecureBoot::On,
        (Some(0), _) => SecureBoot::Off,
        _ => SecureBoot::Unknown,
    }
}

pub fn interpret(probe: Probe, reg: RegistryBootFacts) -> HardwareFacts {
    let firmware = firmware_from(&reg);
    let bitlocker = bitlocker_volumes(&probe);
    let rst_vmd = detect_rst_vmd(&probe.storage_controllers, &probe.disks);
    HardwareFacts {
        is_admin: probe.is_admin,
        os: probe.os,
        cpus: probe.cpu,
        ram_bytes: probe.ram_bytes,
        computer: probe.computer,
        gpus: probe.gpus.iter().map(gpu_from_raw).collect(),
        wifi: wifi_from_raw(&probe.net_adapters),
        disks: probe.disks,
        storage_controllers: probe.storage_controllers,
        firmware,
        secure_boot: secure_boot_from(&reg, firmware),
        bitlocker,
        rst_vmd,
        c_drive: probe.c_drive,
        c_shrinkable: probe.c_shrink.map(|s| s.current.saturating_sub(s.min)),
        fast_startup: reg.hiberboot_enabled.map(|v| v == 1),
        probe_errors: probe.errors,
    }
}

// ---------------------------------------------------------------------------
// OS-touching code (kept thin)
// ---------------------------------------------------------------------------

/// Standard base64 (needed for powershell -EncodedCommand).
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// Runs the read-only probe script. Errors are returned as text so the scan
/// can continue and report "unknown" instead of failing.
pub fn run_probe() -> Result<Probe, String> {
    let utf16: Vec<u8> = PROBE_SCRIPT.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand"])
        .arg(base64(&utf16))
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not start PowerShell: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let json = text.trim().trim_start_matches('\u{feff}');
    serde_json::from_str(json).map_err(|e| {
        format!(
            "hardware probe output was not valid JSON ({e}); stderr: {}",
            String::from_utf8_lossy(&out.stderr).chars().take(400).collect::<String>()
        )
    })
}

#[cfg(windows)]
pub fn read_registry_boot_facts() -> RegistryBootFacts {
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let dword = |path: &str, value: &str| -> Option<u32> {
        hklm.open_subkey_with_flags(path, KEY_READ).ok()?.get_value::<u32, _>(value).ok()
    };
    RegistryBootFacts {
        pe_firmware_type: dword(r"SYSTEM\CurrentControlSet\Control", "PEFirmwareType"),
        secure_boot_enabled: dword(
            r"SYSTEM\CurrentControlSet\Control\SecureBoot\State",
            "UEFISecureBootEnabled",
        ),
        hiberboot_enabled: dword(
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Power",
            "HiberbootEnabled",
        ),
        firmware_type_env: std::env::var("firmware_type").ok(),
    }
}

#[cfg(not(windows))]
pub fn read_registry_boot_facts() -> RegistryBootFacts {
    RegistryBootFacts::default()
}

pub fn scan() -> (HardwareFacts, Option<String>) {
    let reg = read_registry_boot_facts();
    match run_probe() {
        Ok(p) => (interpret(p, reg), None),
        Err(e) => (interpret(Probe::default(), reg), Some(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn parses_probe_with_single_objects_and_nul_letters() {
        let json = r#"{
            "is_admin": false,
            "cpu": {"name":"AMD Ryzen 7","manufacturer":"AuthenticAMD","cores":8,"threads":16},
            "gpus": [{"name":"Meta Virtual Monitor","pnp_id":"ROOT\\DISPLAY\\0000"},
                     {"name":"AMD Radeon RX 7600","pnp_id":"PCI\\VEN_1002&DEV_7480"}],
            "disks": {"number":0,"model":"SSD","size":1000,"bus_type":"NVMe","partitions":
                {"number":2,"drive_letter":"\u0000","size":500}},
            "net_adapters": null,
            "bitlocker_shell": [{"mount":"C:","value":2}],
            "errors": {"c_shrink":"Access denied"}
        }"#;
        let p: Probe = serde_json::from_str(json).unwrap();
        assert_eq!(p.cpu.len(), 1);
        assert_eq!(p.disks[0].partitions[0].drive_letter, None);
        assert!(p.net_adapters.is_empty());
        let hw = interpret(p, RegistryBootFacts { pe_firmware_type: Some(2), secure_boot_enabled: Some(1), hiberboot_enabled: Some(1), firmware_type_env: None });
        assert!(hw.gpus[0].is_virtual);
        assert_eq!(hw.gpus[1].vendor, Vendor::Amd);
        assert!(!hw.gpus[1].is_virtual);
        assert_eq!(hw.firmware, Firmware::Uefi);
        assert_eq!(hw.secure_boot, SecureBoot::On);
        assert_eq!(hw.bitlocker[0].state, BitLocker::Off);
        assert_eq!(hw.fast_startup, Some(true));
        assert_eq!(hw.c_shrinkable, None);
    }

    #[test]
    fn gpu_vendor_from_pci_or_name() {
        let g = |name: &str, pnp: &str| gpu_from_raw(&RawGpu { name: Some(name.into()), pnp_id: Some(pnp.into()), driver_version: None });
        assert_eq!(g("NVIDIA GeForce RTX 3060", "PCI\\VEN_10DE&DEV_2504").vendor, Vendor::Nvidia);
        assert_eq!(g("Intel(R) UHD Graphics 630", "PCI\\VEN_8086&DEV_3E92").vendor, Vendor::Intel);
        assert_eq!(g("GeForce thing", "").vendor, Vendor::Nvidia);
        assert_eq!(g("Microsoft Basic Display Adapter", "ROOT\\BasicDisplay").vendor, Vendor::Other);
    }

    #[test]
    fn rst_vmd_detection() {
        let c = |name: &str, svc: &str| StorageController { name: Some(name.into()), service: Some(svc.into()), ..Default::default() };
        let clean = [c("Standard NVM Express Controller", "stornvme"), c("Microsoft Storage Spaces Controller", "spaceport"), c("Standard SATA AHCI Controller", "storahci")];
        assert!(!detect_rst_vmd(&clean, &[]).detected);
        assert!(detect_rst_vmd(&[c("Intel RST VMD Controller 9A0B", "iaStorVD")], &[]).detected);
        assert!(detect_rst_vmd(&[c("Intel(R) Chipset SATA/PCIe RST Premium Controller", "iaStorAC")], &[]).detected);
        let raid_disk = Disk { number: 0, bus_type: Some("RAID".into()), ..Default::default() };
        assert!(detect_rst_vmd(&clean, &[raid_disk]).detected);
    }

    #[test]
    fn bitlocker_codes() {
        assert_eq!(bitlocker_from_shell_code(Some(2)).0, BitLocker::Off);
        for code in [1, 3, 4, 5, 6, 8] {
            assert_eq!(bitlocker_from_shell_code(Some(code)).0, BitLocker::Active, "code {code}");
        }
        assert_eq!(bitlocker_from_shell_code(None).0, BitLocker::Unknown);
        assert_eq!(bitlocker_from_shell_code(Some(0)).0, BitLocker::Unknown);
    }

    #[test]
    fn bitlocker_cmdlet_wins_over_shell() {
        let p = Probe {
            bitlocker_cmdlet: vec![BitLockerCmdlet { mount: Some("C:".into()), protection: Some("Off".into()), status: Some("FullyEncrypted".into()), percent: Some(100.0) }],
            bitlocker_shell: vec![BitLockerShell { mount: Some("C:".into()), value: Some(2) }],
            ..Default::default()
        };
        // Protection "Off" but still encrypted = suspended = still active.
        assert_eq!(bitlocker_volumes(&p)[0].state, BitLocker::Active);
    }

    #[test]
    fn secure_boot_states() {
        let r = |fw, sb| RegistryBootFacts { pe_firmware_type: fw, secure_boot_enabled: sb, hiberboot_enabled: None, firmware_type_env: None };
        assert_eq!(secure_boot_from(&r(Some(2), Some(0)), Firmware::Uefi), SecureBoot::Off);
        assert_eq!(secure_boot_from(&r(Some(2), None), Firmware::Uefi), SecureBoot::Unknown);
        assert_eq!(secure_boot_from(&r(Some(1), None), Firmware::Bios), SecureBoot::NotSupported);
        // No PEFirmwareType (seen on a real Win10 LTSC machine): fall back to the env var.
        let env = |v: &str| RegistryBootFacts { firmware_type_env: Some(v.into()), ..Default::default() };
        assert_eq!(firmware_from(&env("UEFI")), Firmware::Uefi);
        assert_eq!(firmware_from(&env("Legacy")), Firmware::Bios);
        assert_eq!(firmware_from(&RegistryBootFacts::default()), Firmware::Unknown);
    }

    #[test]
    fn wifi_vendor_notes() {
        let a = RawNetAdapter { description: Some("Intel(R) Wi-Fi 6 AX201 160MHz".into()), media: Some("9".into()), pnp_id: Some("PCI\\VEN_8086&DEV_A0F0".into()), ..Default::default() };
        let eth = RawNetAdapter { description: Some("Realtek PCIe GbE Family Controller".into()), media: Some("14".into()), ..Default::default() };
        let w = wifi_from_raw(&[a, eth]);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].chipset_vendor, "Intel");
    }
}

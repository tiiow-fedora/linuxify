//! VM trial (Stage 3, option A): boot a verified ISO in a virtual machine.
//! Zero risk to the real disks: the VM has no access to them, and by
//! default it has no virtual hard disk at all (live session only).
//!
//! Commands are planned as data (pure, tested) and only then executed, so
//! `--dry-run` prints exactly what would run.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Every VM Linuxify creates carries this prefix; it never touches others.
pub const VM_PREFIX: &str = "Linuxify trial - ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    VirtualBox,
    HyperV,
    Qemu,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::VirtualBox => "VirtualBox",
            Backend::HyperV => "Hyper-V",
            Backend::Qemu => "QEMU",
        }
    }
    pub fn parse(s: &str) -> Option<Backend> {
        match s.to_ascii_lowercase().as_str() {
            "virtualbox" | "vbox" => Some(Backend::VirtualBox),
            "hyperv" | "hyper-v" => Some(Backend::HyperV),
            "qemu" => Some(Backend::Qemu),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Detected {
    pub backend: Backend,
    /// Executable to run (VBoxManage.exe, qemu-system-x86_64.exe, powershell).
    pub exe: PathBuf,
    /// Present but not usable right now, with the reason.
    pub unusable: Option<String>,
}

/// One command to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub what: String,
    pub exe: PathBuf,
    pub args: Vec<String>,
    /// Failure of this step is fine (e.g. "power off if running").
    pub may_fail: bool,
    /// Start it and don't wait (GUI windows).
    pub detach: bool,
    /// Retry this many times, 2 s apart, if it fails (e.g. a VM that is
    /// still unlocking just after power-off).
    pub retries: u32,
}

impl Step {
    fn new(what: &str, exe: &Path, args: &[&str]) -> Step {
        Step { what: what.into(), exe: exe.to_path_buf(), args: args.iter().map(|s| s.to_string()).collect(), may_fail: false, detach: false, retries: 0 }
    }
    pub fn display(&self) -> String {
        let q = |s: &str| if s.contains(' ') || s.is_empty() { format!("\"{s}\"") } else { s.to_string() };
        let mut out = q(&self.exe.display().to_string());
        for a in &self.args {
            out.push(' ');
            out.push_str(&q(a));
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct TrialSpec {
    pub distro_id: String,
    pub iso: PathBuf,
    pub memory_mb: u32,
    pub cpus: u32,
    /// Optional virtual disk (GB) so the installer can be tried too.
    pub disk_gb: Option<u32>,
    /// %LOCALAPPDATA%\Linuxify\vms
    pub vm_dir: PathBuf,
}

pub fn vm_name(distro_id: &str) -> String {
    format!("{VM_PREFIX}{distro_id}")
}

/// Picks VM memory/CPUs that leave the host comfortable.
pub fn size_for_host(host_ram_gb: Option<f64>, host_threads: Option<u32>) -> (u32, u32) {
    let mem = match host_ram_gb {
        Some(r) if r >= 15.0 => 4096,
        Some(r) if r >= 7.5 => 3072,
        Some(_) => 2048,
        None => 3072,
    };
    let cpus = host_threads.map(|t| (t / 4).clamp(2, 4)).unwrap_or(2);
    (mem, cpus)
}

/// VirtualBox OS type for the guest (only IDs present in VirtualBox 7).
pub fn vbox_ostype(distro_id: &str) -> &'static str {
    match distro_id {
        id if id.contains("ubuntu") || id.starts_with("linux-mint") || id == "pop-os" || id == "zorin-os" || id == "elementary-os" || id == "linux-lite" => "Ubuntu_64",
        id if id.starts_with("fedora") || id == "nobara" || id == "bazzite" => "Fedora_64",
        id if id == "debian" || id == "mx-linux" || id == "antix" => "Debian_64",
        id if id == "arch" || id == "endeavouros" || id == "manjaro" || id == "cachyos" || id == "garuda" => "ArchLinux_64",
        _ => "Linux_64",
    }
}

pub fn plan_virtualbox(vbox: &Path, s: &TrialSpec, vm_exists: bool) -> Vec<Step> {
    let name = vm_name(&s.distro_id);
    let iso = s.iso.display().to_string();
    let base = s.vm_dir.display().to_string();
    let mem = s.memory_mb.to_string();
    let cpus = s.cpus.to_string();
    let mut steps = Vec::new();
    if !vm_exists {
        steps.push(Step::new("Create the trial VM", vbox, &["createvm", "--name", &name, "--ostype", vbox_ostype(&s.distro_id), "--basefolder", &base, "--register"]));
        steps.push(Step::new(
            "Set memory, CPUs, UEFI firmware, graphics, network",
            vbox,
            &["modifyvm", &name, "--memory", &mem, "--cpus", &cpus, "--firmware", "efi", "--graphicscontroller", "vmsvga", "--vram", "128", "--nic1", "nat", "--mouse", "usbtablet", "--boot1", "dvd", "--boot2", "disk", "--boot3", "none", "--boot4", "none"],
        ));
        steps.push(Step::new("Add a SATA controller", vbox, &["storagectl", &name, "--name", "SATA", "--add", "sata", "--controller", "IntelAhci", "--portcount", "2"]));
        if let Some(gb) = s.disk_gb {
            let vdi = s.vm_dir.join(&name).join("trial-disk.vdi").display().to_string();
            let mb = (gb * 1024).to_string();
            steps.push(Step::new("Create an empty virtual disk (inside the VM folder only)", vbox, &["createmedium", "disk", "--filename", &vdi, "--size", &mb, "--format", "VDI"]));
            steps.push(Step::new("Attach the virtual disk", vbox, &["storageattach", &name, "--storagectl", "SATA", "--port", "1", "--device", "0", "--type", "hdd", "--medium", &vdi]));
        }
    }
    steps.push(Step::new("Insert the verified ISO", vbox, &["storageattach", &name, "--storagectl", "SATA", "--port", "0", "--device", "0", "--type", "dvddrive", "--medium", &iso]));
    let mut start = Step::new("Start the VM in a window", vbox, &["startvm", &name, "--type", "gui"]);
    start.detach = false; // VBoxManage startvm returns once the window is up
    steps.push(start);
    steps
}

pub fn plan_virtualbox_remove(vbox: &Path, distro_id: &str) -> Vec<Step> {
    let name = vm_name(distro_id);
    let mut off = Step::new("Power off the VM if it is running", vbox, &["controlvm", &name, "poweroff"]);
    off.may_fail = true;
    // Eject the ISO first: `unregistervm --delete` removes attached *hard
    // disk* images, and we never want it near an ISO the user owns.
    let mut eject = Step::new("Eject the ISO", vbox, &["storageattach", &name, "--storagectl", "SATA", "--port", "0", "--device", "0", "--type", "dvddrive", "--medium", "emptydrive"]);
    eject.may_fail = true;
    // VirtualBox keeps the VM locked for a few seconds after power-off.
    let mut eject = eject;
    eject.retries = 10;
    let mut delete = Step::new("Delete the trial VM and its own files", vbox, &["unregistervm", &name, "--delete"]);
    delete.retries = 10;
    vec![off, eject, delete]
}

pub fn plan_qemu(qemu: &Path, s: &TrialSpec) -> Vec<Step> {
    let name = vm_name(&s.distro_id);
    let mem = s.memory_mb.to_string();
    let cpus = s.cpus.to_string();
    let iso = s.iso.display().to_string();
    // whpx = Windows Hypervisor Platform; QEMU falls back to tcg (slow) if unavailable.
    let mut args: Vec<String> = ["-name", &name, "-machine", "q35", "-accel", "whpx,kernel-irqchip=off", "-accel", "tcg", "-m", &mem, "-smp", &cpus, "-cdrom", &iso, "-boot", "d", "-usb", "-device", "usb-tablet", "-vga", "std", "-nic", "user,model=virtio-net-pci"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut steps = Vec::new();
    if let Some(gb) = s.disk_gb {
        let img = s.vm_dir.join(format!("{}.qcow2", s.distro_id));
        let qemu_img = qemu.with_file_name("qemu-img.exe");
        let mut mk = Step::new("Create an empty virtual disk (inside the VM folder only)", &qemu_img, &["create", "-f", "qcow2", &img.display().to_string(), &format!("{gb}G")]);
        mk.may_fail = true; // fine if it already exists
        steps.push(mk);
        args.push("-drive".into());
        args.push(format!("file={},format=qcow2,if=virtio", img.display()));
    }
    steps.push(Step { what: "Start QEMU in a window".into(), exe: qemu.to_path_buf(), args, may_fail: false, detach: true, retries: 0 });
    steps
}

/// PowerShell script for a Hyper-V Generation 2 VM. Needs admin or the
/// "Hyper-V Administrators" group.
pub fn plan_hyperv(powershell: &Path, s: &TrialSpec, vm_exists: bool) -> Vec<Step> {
    let name = vm_name(&s.distro_id).replace('\'', "");
    let iso = s.iso.display().to_string().replace('\'', "''");
    let base = s.vm_dir.display().to_string().replace('\'', "''");
    let mut script = String::from("$ErrorActionPreference='Stop'; ");
    if !vm_exists {
        script += &format!("New-VM -Name '{name}' -Generation 2 -MemoryStartupBytes {}MB -NoVHD -Path '{base}' -SwitchName 'Default Switch' | Out-Null; ", s.memory_mb);
        script += &format!("Set-VMProcessor -VMName '{name}' -Count {}; ", s.cpus);
        // Linux guests need the Microsoft UEFI CA template to boot with Secure Boot on.
        script += &format!("Set-VMFirmware -VMName '{name}' -EnableSecureBoot On -SecureBootTemplate MicrosoftUEFICertificateAuthority; ");
        script += &format!("Add-VMDvdDrive -VMName '{name}'; ");
        if let Some(gb) = s.disk_gb {
            let vhd = s.vm_dir.join(format!("{}.vhdx", s.distro_id)).display().to_string().replace('\'', "''");
            script += &format!("New-VHD -Path '{vhd}' -SizeBytes {gb}GB -Dynamic | Out-Null; Add-VMHardDiskDrive -VMName '{name}' -Path '{vhd}'; ");
        }
    }
    script += &format!("Set-VMDvdDrive -VMName '{name}' -Path '{iso}'; ");
    script += &format!("Set-VMFirmware -VMName '{name}' -FirstBootDevice (Get-VMDvdDrive -VMName '{name}'); ");
    script += &format!("Start-VM -Name '{name}'; Start-Process vmconnect.exe -ArgumentList 'localhost','\"{name}\"'");
    vec![Step::new("Create/start the Hyper-V VM", powershell, &["-NoProfile", "-NonInteractive", "-Command", &script])]
}

// ---------------------------------------------------------------------------
// Detection and execution (OS-touching, thin)
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn reg_string(path: &str, value: &str) -> Option<String> {
    use winreg::enums::*;
    winreg::RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(path).ok()?.get_value(value).ok()
}
#[cfg(not(windows))]
fn reg_string(_: &str, _: &str) -> Option<String> {
    None
}

fn on_path(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(exe)).find(|c| c.is_file()))
}

pub fn detect(is_admin: bool) -> Vec<Detected> {
    let mut out = Vec::new();
    let vbox = reg_string(r"SOFTWARE\Oracle\VirtualBox", "InstallDir")
        .map(|d| PathBuf::from(d).join("VBoxManage.exe"))
        .filter(|p| p.is_file())
        .or_else(|| Some(PathBuf::from(r"C:\Program Files\Oracle\VirtualBox\VBoxManage.exe")).filter(|p| p.is_file()));
    if let Some(exe) = vbox {
        out.push(Detected { backend: Backend::VirtualBox, exe, unusable: None });
    }
    if reg_string(r"SYSTEM\CurrentControlSet\Services\vmms", "ImagePath").is_some() {
        let unusable = (!is_admin).then(|| "Hyper-V is installed, but creating VMs needs an administrator terminal (or membership of 'Hyper-V Administrators').".to_string());
        out.push(Detected { backend: Backend::HyperV, exe: PathBuf::from("powershell.exe"), unusable });
    }
    let qemu = on_path("qemu-system-x86_64.exe").or_else(|| Some(PathBuf::from(r"C:\Program Files\qemu\qemu-system-x86_64.exe")).filter(|p| p.is_file()));
    if let Some(exe) = qemu {
        out.push(Detected { backend: Backend::Qemu, exe, unusable: None });
    }
    out
}

pub fn vbox_vm_exists(vbox: &Path, distro_id: &str) -> bool {
    std::process::Command::new(vbox)
        .args(["showvminfo", &vm_name(distro_id), "--machinereadable"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn hyperv_vm_exists(distro_id: &str) -> bool {
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &format!("if (Get-VM -Name '{}' -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}", vm_name(distro_id))])
        .status()
        .is_ok_and(|s| s.success())
}

/// Runs steps in order, stopping at the first failure that isn't allowed.
pub fn execute(steps: &[Step]) -> Result<(), String> {
    for s in steps {
        println!("  - {}", s.what);
        let mut cmd = std::process::Command::new(&s.exe);
        cmd.args(&s.args);
        if s.detach {
            cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            cmd.spawn().map_err(|e| format!("could not start {}: {e}", s.exe.display()))?;
            continue;
        }
        let mut out = cmd.output().map_err(|e| format!("could not run {}: {e}", s.exe.display()))?;
        let mut tries = 0;
        while !out.status.success() && tries < s.retries {
            tries += 1;
            std::thread::sleep(std::time::Duration::from_secs(2));
            out = cmd.output().map_err(|e| format!("could not run {}: {e}", s.exe.display()))?;
        }
        if !out.status.success() && !s.may_fail {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("'{}' failed:\n    {}\n    {}", s.what, s.display(), err.trim().lines().take(6).collect::<Vec<_>>().join("\n    ")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(disk: Option<u32>) -> TrialSpec {
        TrialSpec {
            distro_id: "linux-mint".into(),
            iso: PathBuf::from(r"C:\Users\X\AppData\Local\Linuxify\isos\linuxmint-22.3-cinnamon-64bit.iso"),
            memory_mb: 4096,
            cpus: 4,
            disk_gb: disk,
            vm_dir: PathBuf::from(r"C:\Users\X\AppData\Local\Linuxify\vms"),
        }
    }
    const VB: &str = r"C:\Program Files\Oracle\VirtualBox\VBoxManage.exe";

    #[test]
    fn virtualbox_new_vm_plan() {
        let p = plan_virtualbox(Path::new(VB), &spec(None), false);
        let cmds: Vec<&str> = p.iter().map(|s| s.args[0].as_str()).collect();
        assert_eq!(cmds, ["createvm", "modifyvm", "storagectl", "storageattach", "startvm"]);
        assert!(p[0].args.contains(&"Linuxify trial - linux-mint".to_string()));
        assert!(p[0].args.contains(&"Ubuntu_64".to_string()));
        assert!(p[1].args.windows(2).any(|w| w == ["--firmware", "efi"]));
        assert!(p[1].args.windows(2).any(|w| w == ["--memory", "4096"]));
        // No hard disk unless asked for.
        assert!(!p.iter().any(|s| s.args.contains(&"hdd".to_string())));
        assert!(p[3].args.contains(&spec(None).iso.display().to_string()));
    }

    #[test]
    fn virtualbox_disk_lives_in_vm_folder() {
        let p = plan_virtualbox(Path::new(VB), &spec(Some(25)), false);
        let mk = p.iter().find(|s| s.args[0] == "createmedium").unwrap();
        assert!(mk.args.iter().any(|a| a.starts_with(r"C:\Users\X\AppData\Local\Linuxify\vms\")));
        assert!(mk.args.windows(2).any(|w| w == ["--size", "25600"]));
    }

    #[test]
    fn existing_vm_only_swaps_iso_and_starts() {
        let p = plan_virtualbox(Path::new(VB), &spec(Some(25)), true);
        let cmds: Vec<&str> = p.iter().map(|s| s.args[0].as_str()).collect();
        assert_eq!(cmds, ["storageattach", "startvm"]);
    }

    #[test]
    fn remove_ejects_iso_before_delete_and_only_touches_our_vm() {
        let p = plan_virtualbox_remove(Path::new(VB), "arch");
        assert_eq!(p.len(), 3);
        assert!(p[1].args.contains(&"emptydrive".to_string()));
        assert_eq!(p[2].args, ["unregistervm", "Linuxify trial - arch", "--delete"]);
        assert!(p.iter().all(|s| s.args[1].starts_with(VM_PREFIX)));
        assert!(p[2].retries > 0, "delete must wait for the VM to unlock");
    }

    #[test]
    fn qemu_plan() {
        let p = plan_qemu(Path::new(r"C:\Program Files\qemu\qemu-system-x86_64.exe"), &spec(None));
        assert_eq!(p.len(), 1);
        assert!(p[0].detach);
        assert!(p[0].args.windows(2).any(|w| w[0] == "-cdrom" && w[1].ends_with(".iso")));
        let with_disk = plan_qemu(Path::new(r"C:\Program Files\qemu\qemu-system-x86_64.exe"), &spec(Some(20)));
        assert_eq!(with_disk[0].exe, PathBuf::from(r"C:\Program Files\qemu\qemu-img.exe"));
    }

    #[test]
    fn hyperv_plan_uses_linux_secure_boot_template() {
        let p = plan_hyperv(Path::new("powershell.exe"), &spec(None), false);
        let script = p[0].args.last().unwrap();
        assert!(script.contains("-Generation 2"));
        assert!(script.contains("MicrosoftUEFICertificateAuthority"));
        assert!(!script.contains("New-VHD"));
    }

    #[test]
    fn sizing() {
        assert_eq!(size_for_host(Some(31.9), Some(32)), (4096, 4));
        assert_eq!(size_for_host(Some(8.0), Some(4)), (3072, 2));
        assert_eq!(size_for_host(Some(4.0), None), (2048, 2));
        assert_eq!(size_for_host(None, None), (3072, 2));
    }

    #[test]
    fn display_quotes_spaces() {
        let s = Step::new("x", Path::new(VB), &["startvm", "Linuxify trial - arch"]);
        assert_eq!(s.display(), format!("\"{VB}\" startvm \"Linuxify trial - arch\""));
    }

    #[test]
    fn backend_names() {
        assert_eq!(Backend::parse("VirtualBox"), Some(Backend::VirtualBox));
        assert_eq!(Backend::parse("hyper-v"), Some(Backend::HyperV));
        assert_eq!(Backend::parse("vmware"), None);
    }
}

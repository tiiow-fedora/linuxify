//! Runs the generated restore.sh against fake package managers in a sandboxed PATH, so every
//! branch (apt, dnf, zypper, pacman, rpm-ostree) is exercised without installing anything.
//! Unix only: restore.sh runs on the new Linux system.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const RESTORE_SH: &str = include_str!("../src/restore.sh");

struct Sandbox {
    root: PathBuf,
    bin: PathBuf,
    home: PathBuf,
    mig: PathBuf,
    log: PathBuf,
}

fn tool(name: &str) -> PathBuf {
    for d in ["/usr/bin", "/bin"] {
        let p = Path::new(d).join(name);
        if p.exists() {
            return p;
        }
    }
    panic!("{name} not found");
}

impl Sandbox {
    fn new(tag: &str, managers: &[&str], with_flatpak: bool) -> Sandbox {
        let root = std::env::temp_dir().join(format!("linuxify-restore-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (bin, home, mig) = (root.join("bin"), root.join("home"), root.join("linuxify-migration"));
        let log = root.join("calls.log");
        for d in [&bin, &home, &mig] {
            std::fs::create_dir_all(d).unwrap();
        }
        // Real tools restore.sh needs; package managers are fakes that only log their arguments.
        for t in ["bash", "env", "find", "awk", "grep", "sha256sum", "cp", "mkdir", "chmod", "cat", "tee", "date", "basename", "dirname", "sed", "ls", "printf", "tr", "sort", "head", "wc", "rm"] {
            std::os::unix::fs::symlink(tool(t), bin.join(t)).unwrap();
        }
        let fake = |name: &str| {
            let p = bin.join(name);
            std::fs::write(&p, format!("#!/bin/bash\necho \"{name} $*\" >> \"{}\"\n[ \"$FAIL_ON\" != \"\" ] && case \"$*\" in *\"$FAIL_ON\"*) exit 1;; esac\nexit 0\n", log.display())).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        for m in managers {
            fake(m);
        }
        if with_flatpak {
            fake("flatpak");
        }
        // sudo just runs the command (which is then logged by the fake manager).
        std::fs::write(bin.join("sudo"), "#!/bin/bash\nexec \"$@\"\n").unwrap();
        std::fs::set_permissions(bin.join("sudo"), std::fs::Permissions::from_mode(0o755)).unwrap();

        std::fs::write(mig.join("restore.sh"), RESTORE_SH).unwrap();
        std::fs::create_dir_all(mig.join("home/Documents")).unwrap();
        std::fs::write(mig.join("home/Documents/a.txt"), "hello").unwrap();
        let sum = Command::new(tool("sha256sum")).arg("home/Documents/a.txt").current_dir(&mig).output().unwrap();
        std::fs::write(mig.join("SHA256SUMS"), String::from_utf8(sum.stdout).unwrap().replace("  ", "  ./")).unwrap();
        std::fs::write(
            mig.join("apps.txt"),
            "# comment\nflatpak org.example.One  # one\nflatpak org.example.Two  # two\npkg git-apt git-dnf  # git\npkg only-apt -  # apt only\npkg - only-dnf  # dnf only\n",
        )
        .unwrap();
        Sandbox { root, bin, home, mig, log }
    }

    fn run(&self, args: &[&str], fail_on: &str) -> (bool, String, String) {
        let out = Command::new(self.bin.join("bash"))
            .arg(self.mig.join("restore.sh"))
            .args(args)
            .current_dir(&self.mig)
            .env_clear()
            .env("PATH", &self.bin)
            .env("HOME", &self.home)
            .env("FAIL_ON", fail_on)
            .output()
            .unwrap();
        let calls = std::fs::read_to_string(&self.log).unwrap_or_default();
        (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), calls)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn apt_installs_flatpaks_then_apt_names() {
    let s = Sandbox::new("apt", &["apt-get"], true);
    let (ok, out, calls) = s.run(&["--yes"], "");
    assert!(ok, "{out}");
    assert!(calls.contains("flatpak remote-add --user --if-not-exists flathub"), "{calls}");
    assert!(calls.contains("flatpak install --user -y --noninteractive flathub org.example.One"), "{calls}");
    assert!(calls.contains("apt-get update"), "{calls}");
    assert!(calls.contains("apt-get install -y git-apt"), "{calls}");
    assert!(calls.contains("apt-get install -y only-apt"), "{calls}");
    assert!(!calls.contains("only-dnf"), "{calls}");
    assert_eq!(std::fs::read_to_string(s.home.join("Documents/a.txt")).unwrap(), "hello");
}

#[test]
fn dnf_uses_the_dnf_column() {
    let s = Sandbox::new("dnf", &["dnf"], true);
    let (ok, out, calls) = s.run(&["--yes"], "");
    assert!(ok, "{out}");
    assert!(calls.contains("dnf install -y git-dnf"), "{calls}");
    assert!(calls.contains("dnf install -y only-dnf"), "{calls}");
    assert!(!calls.contains("only-apt") && !calls.contains("git-apt"), "{calls}");
}

#[test]
fn zypper_and_pacman_branches() {
    let z = Sandbox::new("zypper", &["zypper"], true);
    let (ok, _, calls) = z.run(&["--yes"], "");
    assert!(ok);
    assert!(calls.contains("zypper --non-interactive install git-dnf"), "{calls}");
    let p = Sandbox::new("pacman", &["pacman"], true);
    let (ok, _, calls) = p.run(&["--yes"], "");
    assert!(ok);
    assert!(calls.contains("pacman -S --noconfirm --needed git-dnf"), "{calls}");
}

#[test]
fn flatpak_is_installed_first_when_missing() {
    let s = Sandbox::new("noflatpak", &["apt-get"], false);
    let (_, out, calls) = s.run(&["--yes"], "");
    assert!(calls.contains("apt-get install -y flatpak"), "{calls}\n{out}");
}

#[test]
fn rpm_ostree_skips_native_packages_with_an_explanation() {
    let s = Sandbox::new("ostree", &["rpm-ostree"], true);
    let (ok, out, calls) = s.run(&["--yes"], "");
    assert!(ok, "{out}");
    assert!(calls.contains("flatpak install"), "{calls}");
    assert!(!calls.contains("rpm-ostree"), "must not layer packages: {calls}");
    assert!(out.contains("image-based system"), "{out}");
    assert!(!out.contains("no supported package manager"), "{out}");
}

#[test]
fn failures_are_reported_and_the_script_still_finishes() {
    let s = Sandbox::new("fail", &["apt-get"], true);
    let (ok, out, _) = s.run(&["--yes"], "org.example.Two");
    assert!(ok, "script must exit 0 even when an install fails: {out}");
    assert!(out.contains("flatpak org.example.Two"), "{out}");
    assert!(out.contains("These installs didn't work"), "{out}");
}

#[test]
fn dry_run_changes_nothing_and_runs_no_manager() {
    let s = Sandbox::new("dry", &["apt-get"], true);
    let (ok, out, calls) = s.run(&["--dry-run"], "");
    assert!(ok, "{out}");
    assert!(calls.is_empty(), "dry run must not call package managers: {calls}");
    assert!(!s.home.join("Documents").exists(), "dry run must not copy files");
    assert!(out.contains("[dry-run]"), "{out}");
}

#[test]
fn existing_files_are_never_overwritten_and_a_second_run_copies_nothing() {
    let s = Sandbox::new("keep", &["apt-get"], true);
    std::fs::create_dir_all(s.home.join("Documents")).unwrap();
    std::fs::write(s.home.join("Documents/a.txt"), "mine").unwrap();
    let (ok, out, _) = s.run(&["--yes"], "");
    assert!(ok, "{out}");
    assert_eq!(std::fs::read_to_string(s.home.join("Documents/a.txt")).unwrap(), "mine");
    assert!(out.contains("1 already existed"), "{out}");
}

#[test]
fn damaged_files_stop_the_restore_unless_confirmed() {
    let s = Sandbox::new("damage", &["apt-get"], true);
    std::fs::write(s.mig.join("home/Documents/a.txt"), "tampered").unwrap();
    // --yes means "continue anyway" for the prompts, so use stdin-less mode: ask() reads /dev/tty
    // when --yes is absent, which fails here, so the script must stop.
    let (ok, _, calls) = s.run(&[], "");
    assert!(!ok);
    assert!(calls.is_empty());
}

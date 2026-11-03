//! The refusal paths of remove-windows.sh, run with fake system tools in a sandboxed PATH.
//! None of these runs can reach a real disk: `lsblk`, `sfdisk`, `mount` and friends are fakes that
//! log their calls, and every test also asserts the destructive tools were never invoked.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCRIPT: &str = include_str!("../src/remove-windows.sh");

fn tool(name: &str) -> PathBuf {
    for d in ["/usr/bin", "/bin"] {
        let p = Path::new(d).join(name);
        if p.exists() {
            return p;
        }
    }
    panic!("{name} not found");
}

struct Sandbox {
    root: PathBuf,
    bin: PathBuf,
    home: PathBuf,
    mig: PathBuf,
    log: PathBuf,
}

impl Sandbox {
    /// `uid`: what `id -u` says. `rootfs`: what `findmnt -o FSTYPE /` says.
    fn new(tag: &str, uid: &str, rootfs: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!("linuxify-rw-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (bin, home, mig) = (root.join("bin"), root.join("home"), root.join("mig"));
        let log = root.join("calls.log");
        for d in [&bin, &home, &mig, &home.join("Documents")] {
            std::fs::create_dir_all(d).unwrap();
        }
        for t in ["bash", "env", "grep", "sed", "awk", "cat", "sha256sum", "mktemp", "rm", "head", "tail", "sort", "tr", "basename", "date", "df", "printf", "wc", "rmdir"] {
            std::os::unix::fs::symlink(tool(t), bin.join(t)).unwrap();
        }
        let write = |name: &str, body: String| {
            let p = bin.join(name);
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let logger = |name: &str| format!("#!/bin/bash\necho \"{name} $*\" >> \"{}\"\nexit 0\n", log.display());
        write("id", format!("#!/bin/bash\necho {uid}\n"));
        // findmnt: "/" file system type is configurable; anything else prints nothing.
        write("findmnt", format!("#!/bin/bash\ncase \"$*\" in *FSTYPE*) echo {rootfs};; esac\nexit 0\n"));
        for destructive in ["sfdisk", "partx", "efibootmgr", "mount", "umount", "mv", "update-grub"] {
            write(destructive, logger(destructive));
        }
        // lsblk: reports no NTFS partitions at all.
        write("lsblk", logger("lsblk"));
        write("sudo", format!("#!/bin/bash\necho \"sudo $*\" >> \"{}\"\nexec \"$@\"\n", log.display()));
        std::fs::write(mig.join("remove-windows.sh"), SCRIPT).unwrap();
        Sandbox { root, bin, home, mig, log }
    }

    /// Restored files in $HOME plus a matching SHA256SUMS.
    fn with_restored_files(&self) {
        std::fs::create_dir_all(self.mig.join("home/Documents")).unwrap();
        for (n, c) in [("a.txt", "alpha"), ("b.txt", "beta")] {
            std::fs::write(self.mig.join("home/Documents").join(n), c).unwrap();
            std::fs::write(self.home.join("Documents").join(n), c).unwrap();
        }
        let sum = Command::new(tool("sha256sum")).args(["./home/Documents/a.txt", "./home/Documents/b.txt"]).current_dir(&self.mig).output().unwrap();
        std::fs::write(self.mig.join("SHA256SUMS"), sum.stdout).unwrap();
    }

    fn run(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(self.bin.join("bash"))
            .arg(self.mig.join("remove-windows.sh"))
            .args(args)
            .current_dir(&self.mig)
            .env_clear()
            .env("PATH", &self.bin)
            .env("HOME", &self.home)
            .output()
            .unwrap();
        (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn destructive_calls(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default().lines().filter(|l| !l.starts_with("lsblk") && !l.starts_with("mount") && !l.starts_with("umount") && !l.starts_with("sudo mount") && !l.starts_with("sudo umount")).collect::<Vec<_>>().join("\n")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn refuses_to_run_as_root() {
    let s = Sandbox::new("root", "0", "ext4");
    s.with_restored_files();
    let (ok, out) = s.run(&["--really"]);
    assert!(!ok);
    assert!(out.contains("REFUSED") && out.contains("normal user"), "{out}");
    assert_eq!(s.destructive_calls(), "");
}

#[test]
fn refuses_in_a_live_session() {
    for fs in ["overlay", "squashfs", "tmpfs"] {
        let s = Sandbox::new(fs, "1000", fs);
        s.with_restored_files();
        let (ok, out) = s.run(&["--really"]);
        assert!(!ok, "{fs}: {out}");
        assert!(out.contains("live USB/installer session"), "{fs}: {out}");
        assert_eq!(s.destructive_calls(), "", "{fs}");
    }
}

#[test]
fn refuses_when_the_checksum_file_is_missing() {
    let s = Sandbox::new("nosums", "1000", "ext4");
    let (ok, out) = s.run(&["--really"]);
    assert!(!ok);
    assert!(out.contains("SHA256SUMS not found"), "{out}");
    assert_eq!(s.destructive_calls(), "");
}

#[test]
fn refuses_when_a_restored_file_was_edited() {
    let s = Sandbox::new("edited", "1000", "ext4");
    s.with_restored_files();
    std::fs::write(s.home.join("Documents/a.txt"), "edited").unwrap();
    let (ok, out) = s.run(&["--really"]);
    assert!(!ok);
    assert!(out.contains("don't match the originals") && out.contains("--accept-changed"), "{out}");
    assert_eq!(s.destructive_calls(), "");
}

#[test]
fn refuses_when_a_restored_file_is_missing_even_with_accept_changed() {
    let s = Sandbox::new("missing", "1000", "ext4");
    s.with_restored_files();
    std::fs::remove_file(s.home.join("Documents/b.txt")).unwrap();
    let (ok, out) = s.run(&["--really", "--accept-changed"]);
    assert!(!ok);
    assert!(out.contains("missing from your home folder"), "{out}");
    assert_eq!(s.destructive_calls(), "");
}

#[test]
fn refuses_when_no_windows_is_found_and_never_deletes() {
    let s = Sandbox::new("nowin", "1000", "ext4");
    s.with_restored_files();
    let (ok, out) = s.run(&["--really"]);
    assert!(!ok);
    assert!(out.contains("2 of 2 files match"), "{out}");
    assert!(out.contains("no Windows installation found"), "{out}");
    assert_eq!(s.destructive_calls(), "");
}

#[test]
fn unknown_options_are_rejected() {
    let s = Sandbox::new("opt", "1000", "ext4");
    let (ok, out) = s.run(&["--force-everything"]);
    assert!(!ok);
    assert!(out.contains("Unknown option"), "{out}");
}

#[test]
fn script_text_keeps_its_safety_properties() {
    // Deletion happens only after the phrase check, and only through sfdisk --delete on listed targets.
    let really = SCRIPT.find("sfdisk --no-reread --delete").unwrap();
    let phrase = SCRIPT.find(r#"[ "$reply" = "$phrase" ] || refuse"#).unwrap();
    assert!(phrase < really, "the typed confirmation must come before any deletion");
    assert!(SCRIPT.contains(r#"[ "$REALLY" != 1 ]"#), "check-only must be the default");
    assert!(!SCRIPT.contains("mkfs") && !SCRIPT.contains("wipefs") && !SCRIPT.contains("dd if="), "no formatting tools");
    assert!(SCRIPT.contains("internal check:") && SCRIPT.contains("is an EFI partition"));
}

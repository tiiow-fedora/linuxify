# Safety model

Linuxify can repartition a disk, and one script it generates can delete Windows. This page says what
it will and won't do, the layers that stand in the way, and how far each has been tested.

> **Back up first**, to a separate disk that you then unplug. Linuxify's staged copy is not a backup.
> Experimental software, no warranty (see `LICENSE`). If you are not comfortable repairing a PC that won't
> boot, use `linuxify try` (a virtual machine) and install Linux from a USB stick yourself.

## The short version

1. Stages 1–4 (`scan`, `choose`, `try`, `migrate`) **never modify disks, partitions or boot settings**.
   They write only under `%LOCALAPPDATA%\Linuxify\` and to the staging folder you choose, and `migrate` only copies.
2. `install` is a **dry run unless you add `--execute`**. `install --undo` and `--fix-boot-entries` are too.
3. A real run needs administrator rights **and** a typed phrase that names the disk and the exact size.
4. Windows is kept. Linuxify never removes it. Removing Windows is a separate script run from Linux, behind its own checks.
5. Linuxify never installs Linux itself. It prepares space and a one-time boot entry for the distro's own installer.

## What each stage may touch

| Stage | Reads | Writes |
|---|---|---|
| `scan` | registry, `winget list`, WMI/PowerShell hardware facts, your folders' sizes and a few profile files | `%LOCALAPPDATA%\Linuxify\reports\` |
| `choose` | latest scan, answers | `choice.json`, `choice.html` in the Linuxify folder |
| `try` | the distro's official checksum file and ISO (HTTPS) | ISO under `%LOCALAPPDATA%\Linuxify\isos\`; a VM named `linuxify-<distro>` in your VM program |
| `migrate` | your files | **copies** into `<target>\linuxify-migration\`. Originals are never moved, changed or deleted |
| `install` (default) | a fresh scan | nothing |
| `install --execute` | a fresh scan | shrinks C:, creates `LNXINST` (and maybe `LNXDATA`), adds one boot entry, writes `install-state.json` |
| `install --undo --execute` | `install-state.json` | removes those partitions and entry, grows C: back |

`try --remove` deletes only the VM Linuxify made for that distro.

## The gates in front of `install --execute`

Before any change, all of these are re-checked on a **fresh** scan taken at that moment, not on an old report.
If any fails, the run stops and prints every reason, not just the first.

| Gate | Why |
|---|---|
| Running as administrator | Disk changes need it; also stops accidental runs |
| No readiness blockers | See below |
| Secure Boot vs the distro | If Secure Boot is on and the distro's installer can't start under it (or Linuxify doesn't know), refuse **before** shrinking, so you are never left with a smaller C: and nothing to boot |
| Not running on battery | A power loss halfway through shrinking C: is the worst case |
| Installer ISO downloaded and verified | The installer partition is filled from a SHA-256-verified ISO |
| A verified staged copy exists (`migrate`) | You get a file-count + checksum check before the disk is touched |
| The staged copy is **not** on the Windows disk | Otherwise one disk failure or mistake takes both |
| No earlier Linuxify install is pending | One install setup at a time; `--undo` it first |
| No `no-install.lock` | See [The lock file](#the-lock-file) |
| Typed confirmation matches | See below |
| RAM large enough for the distro | Checked while planning; the plan refuses before any change |

### Readiness blockers

The scan turns these into blockers, and `install` refuses while any exists:

- **BitLocker is on for C:**, or its state could not be determined (unknown is treated as unsafe, not as "off")
- **Intel RST/VMD (RAID) disk mode**, which hides the disk from most Linux installers
- **Not enough room to shrink C:** (about 50 GB minimum), or free space unreadable
- **Hardware check failed**, or the PC is **ARM**
- Windows in **legacy BIOS** mode or **unknown firmware**, and **unknown Secure Boot** state, are reported so the right boot method can be chosen

Each comes with a "what to do" in the report, for example how to suspend or decrypt BitLocker.

### The typed confirmation

The phrase is built from the plan, e.g. `SHRINK C: ON DISK 0 BY 104 GB`. It changes whenever the plan
changes. If the disk layout shifts between the dry run and the real run, the old phrase no longer matches and
the run is refused: this happened in testing (91 GB became 89 GB) and is intended. Undo uses
`UNDO LINUXIFY ON DISK <n>`; removing Windows uses `DELETE WINDOWS FROM <disk>`.

### The lock file

An empty file at `%LOCALAPPDATA%\Linuxify\no-install.lock` makes Linuxify refuse every disk change on that
PC, from the terminal and from the window, until the file is deleted. Create it on any machine that should
never be repartitioned:

```powershell
New-Item -ItemType File "$env:LOCALAPPDATA\Linuxify\no-install.lock" -Force
```

## Crash safety

`install-state.json` is written **before** the first change, with a phase marker (`started`, `shrunk`,
`files_copied`, `boot_entry_added`, `ready`). If Linuxify or the PC dies midway, the file shows how far it got and
`linuxify install --undo` knows what to remove. Before any change, undo finds partitions by their exact offset and size, not by label alone.

## Removing Windows (separate, from Linux only)

`linuxify install --replace-windows` **deletes nothing**. It only checks whether removal could be unlocked
(migration recorded, manifest non-empty, every staged file present and matching, file count equal, staged copy not on the Windows disk).

The deletion is `remove-windows.sh`, run on the installed Linux. It is a dry run by default, and `--really`:

- refuses as root (it must check *your* home), on a live USB session, and unless exactly one Windows installation is found;
- re-hashes every restored file in your home against the checksums taken on Windows, and refuses on any missing or edited file;
- refuses if any Windows partition is mounted or holds the migration folder;
- deletes only: the Windows partition, the Microsoft reserved partition, the Windows recovery partition and `LNXINST`, all on that one disk. The EFI partition, Linux partitions and other disks are never touched;
- renames `EFI/Microsoft` rather than deleting it;
- requires the typed phrase `DELETE WINDOWS FROM <disk>`.

Its tests assert that the destructive tools are never invoked on any refusal path, and that the script text
contains no `mkfs`, `wipefs` or `dd`. **It cannot be undone.**

## Downloads

ISOs come only from hosts listed in `data/isos.json`. A checksum URL whose host is not in the entry's
`allowed_hosts` makes the file invalid. Checksums are SHA-256, fetched over HTTPS from the official host. They
are **not** GPG-verified. A hash mismatch deletes nothing and stops the run. Pinned release folders
(Fedora, Mint, Ubuntu-family, NixOS, AlmaLinux) go stale when new releases come out.

## The website quiz

If you connect a website quiz, everything it sends is untrusted: capped at 16 KB, strict JSON, validated
against the built-in questions and `distros.json`. Navigation is allow-listed (only `https`/`mailto` open externally),
popups, downloads and permission requests are denied, and local quizzes are served through a custom protocol
instead of `file://`. See [web-quiz.md](web-quiz.md).

## What has and has not been tested

**Tested for real**, in throwaway VirtualBox VMs (Windows 10 22H2, UEFI, Secure Boot on): scan, migrate, the full
`install` → installer → Linux boots path for Mint (Cinnamon and Xfce), Ubuntu, Kubuntu, Xubuntu, Lubuntu, Debian,
Fedora KDE and Workstation, openSUSE Tumbleweed, Bazzite, Arch and Omarchy; `--undo`, including after Linux was
installed; `restore.sh` on Mint; and `remove-windows.sh --really` once end to end.

**Not tested:**

- **Any real hardware.** Not one real PC has been repartitioned by this tool.
- BitLocker-encrypted disks (refused by design), and disks where C: isn't followed by space Windows can shrink into.
- AlmaLinux 10 on a CPU that supports it (the VM's CPU couldn't boot it), and NixOS (refused by design).
- The Hyper-V and QEMU trial backends.
- `restore.sh` package installs on `dnf`, `zypper` and `pacman` (checked against fakes only).
- The UAC "restart as administrator" prompt in the window, and the web-view quiz on Windows.
- Most `app_map.json` entries added in the latest session are from memory, not checked against live sources; `wine_ok` claims are community reports.

Full detail is in `PROGRESS.md`.

## Reporting a problem

If a gate let something through that it shouldn't have, or a message is misleading, please open an issue
with the dry-run output (`linuxify install <distro>` with no `--execute`). It lists disk numbers, sizes and models, so glance through it before posting.

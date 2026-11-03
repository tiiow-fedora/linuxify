# Installing and using Linuxify

## Requirements

- Windows 10 or 11, 64-bit (x64). ARM PCs are reported as a blocker for the install step.
- For `try` (the virtual-machine trial): VirtualBox is the only backend tested for real. Hyper-V and
  QEMU are supported in code but untested.
- For `install` (the real dual boot): UEFI firmware, an administrator terminal, a separate drive for
  your staged files, and a PC that is not BitLocker-encrypted. See the [safety model](safety-model.md).

## Get Linuxify

### Option A: download

1. Download `linuxify.exe` from the
   [latest release](https://github.com/tiiow-fedora/linuxify/releases/latest). It is one
   self-contained file. Put it in a folder of its own.
2. It is **not code-signed**, so Windows SmartScreen may warn on first run. Compare the file with the
   release's `SHA256SUMS.txt` before running it:

   ```powershell
   Get-FileHash .\linuxify.exe -Algorithm SHA256
   ```

   The hash printed must equal the one in `SHA256SUMS.txt`. If it doesn't, delete the file.

### Option B: build it

Install Rust from <https://rustup.rs> (MSVC toolchain), then in the repository:

```powershell
cargo build --release
cargo test
```

The result is `target\release\linuxify.exe`, with the C runtime linked in statically.

The `data\` folder next to the exe (if present) overrides the copy compiled into it, as does
`--data-dir <folder>` for `app_map.json`, `distros.json` and `isos.json`.

## Two ways to run it

- **The window.** Double-click `linuxify.exe` (or run `linuxify gui`). It walks through the five
  steps. `linuxify gui --page install` opens a given page (`welcome`, `scan`, `choose`, `try`,
  `migrate`, `install`).
- **A terminal.** The same code, as commands. The rest of this page uses commands because they are
  easier to write down.

Both use the same safety rules. The window never skips a gate that the terminal has.

## Walkthrough

### 0. Back up first

Before anything else, copy what matters to a **separate disk that you then unplug**. Linuxify's own
staged copy is a convenience, not a backup: it is made by the same tool that may later change your
disk, and the install step refuses to run if the staged copy sits on the Windows disk.

### 1. Scan (read-only)

```powershell
linuxify scan --open
```

Reads installed programs (registry + `winget list`), hardware and boot facts (UEFI/BIOS, Secure
Boot, BitLocker, Intel RST/VMD, free space, Wi-Fi chipset), and the size of your user data. It writes
`report.html` and `report.json` under `%LOCALAPPDATA%\Linuxify\reports\`, with a copy in `reports\latest\`.

The report starts with **blockers** (things that must be fixed before an install), then heads-ups
and info. Each has a plain-English "what to do". Run as administrator for the exact shrinkable size
of C: (a normal run estimates from free space).

Useful flags: `--no-winget` (registry only), `--data-dir <folder>`.

### 2. Choose a distro

```powershell
linuxify choose --open
```

Fourteen plain-language questions, combined with the scan (NVIDIA GPU, RAM, Steam library, Secure
Boot, programs that won't run on Linux). The result is a ranked list with "why it fits" and "good to
know" notes, written to `choice.html` / `choice.json`. Distros that need more RAM than the PC has are left
out, with the reason shown.

- `--answers file.json` reads answers from a file instead of asking (see `tests\profiles\` for ten examples).
- `--no-scan` ignores the latest scan.
- `--web` takes the quiz from a website; see [web-quiz.md](web-quiz.md).

The ratings in `data\distros.json` are judgement calls, not benchmarks.

### 3. Try it (zero risk)

```powershell
linuxify try linux-mint            # download, verify, boot in a virtual machine
linuxify try --list                # which distros can be downloaded, which VM programs were found
linuxify try fedora-kde --disk 25  # also give the VM an empty virtual disk, to try its installer
linuxify try linux-mint --remove   # delete Linuxify's trial VM for that distro (nothing else)
```

The ISO is downloaded from the distro's official host and checked against the distro's published
SHA-256 before it is used. Downloads resume if interrupted. `--dry-run` prints every command.
`--iso-file <path>` uses an ISO you already have, which must match the official hash.

A VM cannot show whether *your* Wi-Fi card or GPU works (it runs on virtual hardware, with Secure
Boot off). A USB live stick is the way to check that.

### 4. Migrate (copies, never moves)

```powershell
linuxify migrate --target E:\ --dry-run   # totals, and what is skipped and why. Writes nothing.
linuxify migrate --target E:\             # copy + verify. Re-run to resume.
linuxify migrate --target E:\ --verify    # just re-check an existing copy
```

Creates `E:\linuxify-migration\` with your files in a Linux-shaped layout and `manifest.json` (every
file's size and SHA-256), `SHA256SUMS`, `restore.sh`, `apps.txt`, `README.txt` and `flagged.txt`.

- Choose a target on a **different physical disk** than Windows: a second internal drive, a USB disk or a NAS.
- Close your browsers first; locked files are flagged and picked up on the next run.
- Chromium-family passwords and cookies cannot be carried over (Windows encrypts them to your login).
  Export them from the browser first; `README.txt` explains how.
- Only English folder names are used (`Documents`, `Pictures`…). On a non-English Linux, files land beside the
  translated folders rather than in them.

### 5. Install (changes your disk)

Read [safety-model.md](safety-model.md) first. Then:

```powershell
linuxify install linux-mint             # show the plan. Changes nothing. This is the default.
```

Read the plan. It lists the disk, the exact shrink, the partitions to be created and every blocker. When
every check passes, from an **administrator** terminal:

```powershell
linuxify install linux-mint --execute
```

You are asked to type a phrase that names the disk and the size, such as
`SHRINK C: ON DISK 0 BY 104 GB`. Nothing happens until it matches exactly.

What it does: shrinks C:, creates a small FAT32 partition `LNXINST` holding the distro's own installer (plus
an NTFS `LNXDATA` partition for some ISOs, such as Kubuntu's, that contain a file too big for FAT32), leaves
the rest **unallocated**, and adds a **one-time** boot entry. It does not install Linux itself.

Restart. The PC boots the installer once. In the distro's installer, install into the free space
(the plan prints the right choice for each installer, for example "Install alongside Windows Boot Manager"
in Ubuntu-family installers). Windows stays.

Options: `--linux-gb <n>` (default 100, or as much as fits safely).

### 6. On the new Linux system

Open the `linuxify-migration` folder (on the drive you staged to) in a terminal:

```bash
bash restore.sh --dry-run    # show what it would do; changes nothing
bash restore.sh              # verify checksums, copy files home, set up browser profiles, install apps
```

`restore.sh` never overwrites an existing file, can be run twice safely, and writes a log to
`~/linuxify-restore.log`. It installs apps from `apps.txt`, Flatpak first, then native packages. Only the
`apt` path has been tested end to end; the `dnf`/`zypper`/`pacman` branches are tested against fakes only.

### 7. Optional, much later: removing Windows

```bash
bash remove-windows.sh       # check only: lists exactly what would be deleted
```

Only after living on Linux for a while, with your files verified and backed up elsewhere. See
[undo.md](undo.md#removing-windows-is-the-one-step-with-no-undo) before using `--really`.

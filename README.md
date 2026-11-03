# Linuxify

A Windows program that helps you move to Linux. It scans your PC, recommends a distro, lets you
try it safely, copies your files into a Linux-ready staging folder, and (last, and only with
explicit confirmation) sets up a dual boot.

> ## ⚠️ Safety warning: read this first
>
> `linuxify install` **repartitions your disk**: it shrinks C:, creates a partition and adds a
> boot entry. `remove-windows.sh` **permanently deletes Windows**. These are experimental tools:
>
> - They have been tested **only in throwaway VirtualBox VMs** (Windows 10 22H2, UEFI), once per
>   offered distro. Never on real hardware. See `PROGRESS.md` for exactly what is and isn't
>   tested.
> - **Back up everything first**, to a separate disk you then unplug. Linuxify's own staged copy
>   is not a backup.
> - `linuxify install` only shows its plan unless you add `--execute`; read the plan first. Real runs
>   need administrator rights and a typed confirmation that names the disk.
> - To make a PC permanently refuse any disk change, create an empty file
>   `%LOCALAPPDATA%\Linuxify\no-install.lock`.
>
> No warranty (see `LICENSE`). If you're not comfortable fixing a PC that won't boot, use
> `linuxify try` (a virtual machine, zero risk) and install Linux from a USB stick yourself.

## Download

Get `linuxify.exe` from the [latest release](https://github.com/tiiow-fedora/linuxify/releases/latest)
(Windows 10/11, x64). It's one self-contained file: put it in a folder and double-click it to
open the window, or run it from a terminal (`linuxify scan --open`). It isn't code-signed, so SmartScreen may warn the first time; compare it
with the release's `SHA256SUMS.txt` (`Get-FileHash linuxify.exe`).

## Build

Or build it yourself. Windows 10/11, x64. Install Rust from <https://rustup.rs> (MSVC toolchain), then:

```
cargo build --release
cargo test
```

The result is one self-contained `target\release\linuxify.exe` (the C runtime is linked statically).

## Use

Every stage is useful on its own. Stages 1–4 never change your disks.

### In a window

Double-click `linuxify.exe` (or run `linuxify gui`) and it walks you through the five steps:
scan, a short quiz, a trial in a virtual machine, copying your files, and the install. It runs the
same code as the commands below, with the same safety rules: the install step always shows its
plan first, lists anything that blocks it, and only starts once you type the phrase that names the
disk; it re-checks everything when you click, and offers Undo afterwards. `linuxify gui --page
install` opens a given step (`welcome`, `scan`, `choose`, `try`, `migrate`, `install`).

| | |
|---|---|
| ![Welcome](docs/gui/1-welcome.png) | ![Scan results](docs/gui/2-scan.png) |
| ![Quiz](docs/gui/3-choose-quiz.png) | ![Distro picks](docs/gui/3-choose-results.png) |
| ![Try it in a VM](docs/gui/4-try.png) | ![What will be copied](docs/gui/5-copy-plan.png) |
| ![Install plan and confirmation](docs/gui/6-install-plan-confirm.png) | ![Install blocked](docs/gui/6-install-blockers.png) |
| ![Install running](docs/gui/6-install-progress.png) | ![Install done, with Undo](docs/gui/6-install-done.png) |

### From a terminal

```
linuxify scan --open                    # 1. read-only scan + Linux readiness report (HTML)
linuxify choose --open                  # 2. ~14 questions -> ranked distro picks
linuxify try linux-mint                 # 3. download + SHA-256-verify the ISO, boot it in a VM
linuxify migrate --target E:\ --dry-run # 4. stage your files for Linux (copy, never move)
linuxify migrate --target E:\

linuxify install linux-mint             # 5. show the dual-boot plan; changes nothing (dry run is the default)
linuxify install linux-mint --execute   #    do it (admin, typed confirmation)
linuxify install --undo --execute       #    remove the installer partitions, grow C: back (without --execute: just shows)
```

What `install` does: shrinks C:, puts the distro's own installer on a small FAT32 partition
(`LNXINST`; plus an NTFS `LNXDATA` partition when an Ubuntu-family ISO has a file too big for
FAT32, like Kubuntu's), leaves the rest unallocated, and adds a **one-time** boot entry. Restart, pick
"Install alongside Windows" in the installer, and finish it there. Windows stays.

On the new Linux system, open the `linuxify-migration` folder (on the drive you staged to) in a
terminal:

```
bash restore.sh --dry-run    # show what it would do
bash restore.sh              # copy your files home, set up profiles, install your apps
bash remove-windows.sh       # OPTIONAL, check only: what removing Windows would delete
```

`remove-windows.sh --really` deletes Windows only after re-checking every restored file against
the checksums taken on Windows, and only after you type the confirmation phrase.

If Linux was installed after the installer partitions and no longer starts after `--undo` (the
firmware boot entry kept an old partition number), `linuxify install --fix-boot-entries --execute`
rewrites the stale numbers (without `--execute` it only lists them).

### Safety

- Everything that changes a disk is a **dry run by default**; it needs `--execute`, an administrator
  shell and a typed confirmation phrase.
- `--undo` re-scans the disk first: it only deletes partitions found at the recorded offset and size,
  refuses to touch anything that looks like a Windows system/boot partition or C:, and grows C: by
  volume letter, not by a saved partition number.
- Creating `%LOCALAPPDATA%\Linuxify\no-install.lock` makes this PC refuse every disk change.
- Before a first run on real hardware, work through [docs/LAPTOP-TEST-CHECKLIST.md](docs/LAPTOP-TEST-CHECKLIST.md).

### Website quiz

`linuxify choose --web` shows a distro questionnaire from a website in its own window (needs a build
with `--features webview` and a quiz address). See `docs/web-quiz.md`.

## Development

```
cargo build
cargo test          # unit tests + script tests; never touches a disk
cargo clippy --all-targets
```

Disk-changing code paths are tested only through unit tests (pure functions that build the
diskpart/bcdedit scripts) and in VMs, never on a development machine.

## Files

- `docs/LAPTOP-TEST-CHECKLIST.md`: step-by-step test plan for the first real-hardware run.
- `PROGRESS.md`: what each stage does, what was verified, and what isn't tested.
- `DECISIONS.md`: design choices and why.
- `data/app_map.json`: about 960 Windows programs mapped to Linux equivalents (format in `data/SCHEMA.md`).
  Pull requests welcome.
- `data/distros.json`, `data/isos.json`: distros, quiz traits, official ISO + checksum sources.
- `HANDOFF.md`: the original build brief.

## Licence

MIT. See `LICENSE`.

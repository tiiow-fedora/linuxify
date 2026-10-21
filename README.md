# Linuxify

A Windows program that helps you move to Linux. It scans your PC, recommends a distro, lets you
try it safely, copies your files into a Linux-ready staging folder, and (last, and only with
explicit confirmation) sets up a dual boot.

> ## ⚠️ Safety warning: read this first
>
> `linuxify install` **repartitions your disk**: it shrinks C:, creates a partition and adds a
> boot entry. `remove-windows.sh` **permanently deletes Windows**. These are experimental tools:
>
> - They have been tested **only in a throwaway VirtualBox VM** (Windows 10 22H2, UEFI + Secure
>   Boot, Linux Mint 22.3). Never on real hardware. See `PROGRESS.md` for exactly what is and
>   isn't tested.
> - **Back up everything first**, to a separate disk you then unplug. Linuxify's own staged copy
>   is not a backup.
> - Always run `--dry-run` first and read the plan. Real runs need a typed confirmation that
>   names the disk.
> - To make a PC permanently refuse any disk change, create an empty file
>   `%LOCALAPPDATA%\Linuxify\no-install.lock`.
>
> No warranty (see `LICENSE`). If you're not comfortable fixing a PC that won't boot, use
> `linuxify try` (a virtual machine, zero risk) and install Linux from a USB stick yourself.

## Build

Windows 10/11, x64. Install Rust from <https://rustup.rs> (MSVC toolchain), then:

```
cargo build --release
cargo test
```

The result is one self-contained `target\release\linuxify.exe` (the C runtime is linked statically).

## Use

Every stage is useful on its own. Stages 1–4 never change your disks.

```
linuxify scan --open                    # 1. read-only scan + Linux readiness report (HTML)
linuxify choose --open                  # 2. ~14 questions -> ranked distro picks
linuxify try linux-mint                 # 3. download + SHA-256-verify the ISO, boot it in a VM
linuxify migrate --target E:\ --dry-run # 4. stage your files for Linux (copy, never move)
linuxify migrate --target E:\

linuxify install linux-mint --dry-run   # 5. show the dual-boot plan; changes nothing
linuxify install linux-mint             #    do it (admin, typed confirmation)
linuxify install --undo                 #    put it back (before installing Linux)
```

What `install` does: shrinks C:, puts the distro's own installer on a small FAT32 partition
(`LNXINST`), leaves the rest unallocated, and adds a **one-time** boot entry. Restart, pick
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

## Files

- `PROGRESS.md`: what each stage does, what was verified, and what isn't tested.
- `DECISIONS.md`: design choices and why.
- `data/app_map.json`: Windows programs mapped to Linux equivalents (format in `data/SCHEMA.md`).
  Pull requests welcome.
- `data/distros.json`, `data/isos.json`: distros, quiz traits, official ISO + checksum sources.
- `HANDOFF.md`: the original build brief.

## Licence

MIT. See `LICENSE`.

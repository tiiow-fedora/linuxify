# FAQ

## Will this delete Windows?

No. `install` keeps Windows and sets up a dual boot. The only code that deletes Windows is
`remove-windows.sh`, which you run yourself from the installed Linux, after checks, with a typed phrase.
See [safety-model.md](safety-model.md).

## Can I try Linux without risk?

Yes: `linuxify try <distro>` boots the official ISO in a virtual machine. Nothing on your disk changes. A VM
can't tell you whether your Wi-Fi or GPU work; a USB live stick can.

## Is it safe?

It is careful, not proven. Every disk-changing path is a dry run by default, needs administrator rights and a
typed phrase, and is re-checked on a fresh scan. But it has been tested only in virtual machines, never on real
hardware. Back up to a separate drive first. See [what has been tested](safety-model.md#what-has-and-has-not-been-tested).

## Why does it say "dry run"? How do I run it for real?

`install` shows its plan unless you add `--execute`. Read the plan, fix any blockers, then run from an
administrator terminal with `--execute` and type the phrase it asks for.

## What does `install` actually do?

Shrinks C:, creates a small FAT32 partition `LNXINST` that holds the distro's installer (plus an NTFS
`LNXDATA` when an ISO has a file over 4 GB), leaves the rest unallocated, and adds a one-time boot entry. You
then finish the installation in the distro's own installer, choosing the free space.

## How much space do I need?

About 50 GB shrinkable on C: at minimum, and the default plan is 100 GB or as much as fits safely
(`--linux-gb` to change). The scan tells you the exact figure if you run it as administrator. Defragmenting
doesn't always free the end of the disk; Windows decides how far C: can shrink.

## Does my PC work with it?

It needs: Windows 10/11 x64, UEFI, no BitLocker on C:, not in Intel RST/VMD (RAID) mode, enough shrinkable
space, and for some distros Secure Boot support or Secure Boot off. Run `linuxify scan --open`: the blockers section says
exactly what to fix.

## What about BitLocker?

Linuxify refuses while BitLocker is on, or when it can't tell. Decrypt the drive (Settings → Privacy &
security → Device encryption, or `manage-bde -off C:`), wait for decryption to finish, and scan again. Keep your
recovery key. Suspending BitLocker is *not* enough: the drive is still encrypted.

## What about Secure Boot?

`data/distros.json` records each distro's status. At the time of writing: **supported** (installer starts with
Secure Boot on) for Mint, Ubuntu-family, Fedora, Debian, openSUSE, Zorin, AlmaLinux and a few others;
**needs key enrolment** (a one-time MOK step after install) for Bazzite, Aurora and Bluefin; **needs Secure
Boot off** for Pop!_OS, Arch-family, Omarchy, NixOS, Void, Gentoo and PikaOS; **unknown** for MX Linux,
elementary OS, Nobara, antiX, Linux Lite and Solus. If Secure Boot is on and the distro can't or may not
start, `install` refuses before shrinking anything. Turn Secure Boot off in firmware, or choose another distro.

## Which distro will it pick?

Fourteen questions plus your scan produce a ranked list of the 36 distros in `data/distros.json`. The ratings are judgement calls, not
benchmarks, and no outside reviewer has checked them. Treat the top three as a shortlist, and `try` them.

## Can I install a distro that isn't downloadable by Linuxify?

You can `choose` any of the 36, but `try` and `install` need an entry in `data/isos.json` (15 distros).
For others, download the ISO yourself and use `--iso-file`; it must still match the official hash, which Linuxify
needs to know.

## What happens to my files?

`migrate` copies, never moves. Originals stay on Windows. The copy has a manifest of every file's SHA-256, and
`restore.sh` verifies it again on Linux before putting files in your home folder.

## What isn't carried over?

Chromium-family (Chrome, Edge, Brave) saved passwords and cookies, which Windows encrypts to your login (export
them first). Windows-only app data. Anything flagged in `flagged.txt`. Bookmarks, Firefox/LibreWolf/Waterfox and
Thunderbird profiles, SSH keys, fonts, VS Code settings and KeePass databases are handled.

## Will my programs work?

`scan` classifies each program as **native** (a Linux version exists), **alternative** (a replacement), **wine_ok**
(runs via Wine/Proton, a community report), **blocked** (known not to work: kernel anti-cheat games, Adobe CC,
some Office features) or **unknown**. Unknown means unknown, not "fine". `data/app_map.json` has about 960
entries; corrections are welcome as pull requests.

## Can I run it without the window?

Yes, every step is a command. `linuxify --help` lists them.

## Does it phone home?

Its only network code downloads an ISO and its checksum file from the distro's official host (`try`, ISO
checks), plus the website quiz if you set one up. It has no telemetry code that we know of: the scan stays on your PC.
You can confirm with `grep -rn reqwest src`.

## Is the .exe signed?

No. SmartScreen may warn. Verify the SHA-256 against the release's `SHA256SUMS.txt`, or build it yourself.

## Does it work on macOS, or install to a Mac/Linux host?

No. It is a Windows program. (The Rust code builds and its tests run on Linux, but disk changes use Windows APIs.)

## Which Linux can't it install?

NixOS is refused by design (its initrd only mounts a true ISO 9660 image). AlmaLinux 10 needs a newer CPU level
(x86-64-v3) and is refused on older CPUs. Anything the window or plan marks as a blocker won't proceed.

## Where are Linuxify's files?

Under `%LOCALAPPDATA%\Linuxify\`: `reports\`, `isos\`, `choice.json`, `install-state.json`, and
optionally `no-install.lock` and `quiz.json`. Your migrated files are wherever you pointed `--target`.

## How do I report a bug or a wrong app mapping?

Open an issue or pull request at <https://github.com/tiiow-fedora/linuxify>. For install problems include the dry-run output.

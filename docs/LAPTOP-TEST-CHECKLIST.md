# Real-hardware test checklist (laptop)

Everything destructive in Linuxify has so far been tested only in VMs. This list is for the
first run on a **real laptop**. Work top to bottom; stop at the first failure and write down what
you saw (the logs live in `%LOCALAPPDATA%\Linuxify\install\*.log`).

> Use a laptop you can afford to reinstall. Do not use your only working computer.

## 0. Before you start (cannot be skipped)

- [ ] A full backup of everything you care about on a **different** drive (not just Linuxify's copy).
- [ ] A Windows recovery USB / the BitLocker recovery key saved off the laptop (`manage-bde -protectors -get C:`).
- [ ] The laptop is on AC power and the battery is above 50%.
- [ ] Note the starting layout: `Get-Disk; Get-Partition` and `bcdedit /enum all` saved to a file on the backup drive.
- [ ] Note the firmware boot entries: `bcdedit /enum firmware` and the BIOS/UEFI boot menu (F12/Esc/F2).
- [ ] BIOS/UEFI: know how to open the boot menu and how to toggle Secure Boot.
- [ ] Windows Fast Startup **off**, BitLocker/Device Encryption suspended or off (Linuxify should warn about both).
- [ ] Intel RST/VMD: SATA mode is AHCI, or you accept that the Linux installer may not see the disk.

## 1. Read-only steps (safe, run first)

- [ ] `linuxify scan --open` completes; the report shows the right CPU, RAM, GPU, disk, Wi-Fi, firmware (UEFI), Secure Boot state.
- [ ] Report warnings match reality (Fast Startup, NVIDIA, BitLocker, RST/VMD).
- [ ] `linuxify choose --open` gives sensible picks for this laptop.
- [ ] `linuxify try <distro>` downloads, verifies the SHA-256, and boots in a VM (needs VirtualBox).
- [ ] `linuxify migrate --target <other drive> --dry-run`, then for real: files are **copied, not moved**; spot-check 10 files by hand.

## 2. Install dry run

- [ ] `linuxify install <distro>` (no `--execute`) prints a plan: shrink amount, LNXINST/LNXDATA sizes, offsets.
- [ ] The plan leaves Windows at least the free space you expect; the offsets look right in Disk Management.
- [ ] Nothing changed: `Get-Partition` output is identical to the saved one.
- [ ] `linuxify install --undo` (dry run) without a prior install says there is nothing to undo.
- [ ] Create `%LOCALAPPDATA%\Linuxify\no-install.lock`: `install --execute` must refuse. Delete it again.

## 3. Install for real (`--execute`)

- [ ] Wrong confirmation phrase: refused, nothing changed. Right phrase: proceeds.
- [ ] After it ends, Disk Management shows C: smaller, new FAT32 `LNXINST` (and `LNXDATA` if planned) with **no drive letter**.
- [ ] `bcdedit /enum firmware`: one-time "Linuxify Linux installer" entry, `bootsequence` points at it.
- [ ] Reboot. The laptop boots the distro installer (if Secure Boot blocks it, record the exact message).
- [ ] Installer sees the disk, offers "install alongside Windows", Wi-Fi works (or note which driver is missing).
- [ ] Finish the Linux install; reboot; the boot menu (F12) lists both Windows and Linux.
- [ ] Windows still boots and its files are intact. Linux boots too.
- [ ] Restart a second time: it must **not** go to the installer again (one-time entry).

## 4. Restore data on Linux

- [ ] Open `linuxify-migration`, `bash restore.sh --dry-run`, then `bash restore.sh`.
- [ ] Checksums verify; files appear in the home folder; Flatpak apps install.
- [ ] `bash remove-windows.sh` **without** `--really` only reports. Do not run `--really` on the first pass.

## 5. Undo (only on a machine where Linux was *not* finished, or after a clean-slate retry)

- [ ] `linuxify install --undo` (dry run) lists: delete boot entry, delete LNXINST (and LNXDATA), `select volume C` + `extend`.
- [ ] Check the partition numbers it would delete against Disk Management. They must be the installer ones only.
- [ ] `linuxify install --undo --execute`, exact phrase typed. Afterwards C: is back to its original size and Windows boots.
- [ ] If Linux **was** installed: afterwards it still boots. If not, run `linuxify install --fix-boot-entries` (dry run, then `--execute`) and compare `bcdedit /enum firmware`.
- [ ] Running `--undo` a second time is harmless (nothing left to delete).

## 6. Interrupted-run drills (optional, advanced)

- [ ] Power off during step 3 after the shrink (use a VM first if possible): next boot, `install --undo` recovers the layout.
- [ ] Disconnect AC mid-plan: install refuses on battery.

## 7. Record results

For each section: pass/fail, laptop model, firmware version, Secure Boot on/off, and attach logs and
photos of the boot menu. Open an issue for every failure.

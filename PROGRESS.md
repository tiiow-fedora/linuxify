# Progress

## Stage 1 — Scanner + Linux Readiness Report: DONE (2026-10-18)

Run it:

```
cargo build --release
target\release\linuxify.exe scan          # add --open to open the report, --no-winget to skip winget
```

Output: `%LOCALAPPDATA%\Linuxify\reports\scan-<time>\report.html` + `report.json`, and a copy in `reports\latest\`.

### What works
- Installed programs: HKLM 64/32-bit and HKCU uninstall keys, merged with `winget list` (which adds
  Store/MSIX apps such as Fortnite and Minecraft). Duplicates are removed, and hidden components,
  updates and hotfixes are filtered out.
- Mapping against `data/app_map.json`: 223 apps plus 20 ignore rules. Statuses are native / alternative /
  wine_ok / blocked / unknown. Steam games get ProtonDB links.
- Hardware/boot: CPU, RAM, GPUs (virtual adapters ignored), disks with their partitions, UEFI/BIOS,
  Secure Boot, BitLocker per volume, Intel RST/VMD detection, C: free space (exact shrink size when
  run as admin), Fast Startup, Wi-Fi chipset with Linux driver notes, and which disk holds the EFI
  partition.
- User data: sizes of Documents/Desktop/Pictures/Music/Videos/Downloads (follows folder
  redirection, skips junctions, counts OneDrive cloud-only files); Chrome/Edge/Brave/Vivaldi
  and Firefox/LibreWolf/Waterfox profiles; Thunderbird; `.kdbx` and `.pst` files inside those folders;
  Steam libraries and games; app data worth noting (SSH keys, VS Code, OBS, .minecraft, Discord…).
- Readiness findings: blockers / heads-ups / info, each with a "what to do" in plain English.
- 40 unit tests, all passing (registry filtering, winget table parsing incl. CRLF, dedupe, mapping,
  data-file validation, probe JSON parsing, RST/VMD, BitLocker codes, Secure Boot, readiness rules,
  profiles.ini / Steam VDF / Chromium Local State parsing, HTML escaping).

### Verified on this PC (non-admin run, ~16 s)
- 165 programs: 74 native, 33 alternative, 4 Wine/Proton, 2 blocked (anti-cheat games), 52 unknown.
  Most of the unknowns are Steam games, which all have ProtonDB links.
- **17 programs checked by hand**, all correct: 7-Zip, Discord, Steam, DaVinci Resolve, OBS Studio,
  utilities, dev tools, a VPN client, a media player, peripheral software and several
  Steam/Store games (a blocked anti-cheat game, unknown ones with ProtonDB links).
- Hardware cross-checked against Windows' own tools: AMD CPU, 32 GB RAM, AMD Radeon GPU
  (AMD), UEFI, Secure Boot off, BitLocker off on C: and D:, no RST/VMD, Fast Startup on.
  The report also correctly found that C: is on disk 0 while the EFI partition is on disk 1.
- Folder sizes and file counts match PowerShell's own totals exactly (Documents,
  Downloads, Videos). Steam games found match the manifest files on disk.
- Bugs found by running it for real and then fixed: winget output uses CRLF line endings (the parser
  was dropping every line); registry names with NUL padding created duplicates; a raw `0x99` (™)
  byte stopped a game's name matching; `PEFirmwareType` is missing on this PC, so UEFI showed as unknown.

### Not tested / known limits
- **Not tested on an elevated (admin) run.** That path uses `Get-BitLockerVolume` and
  `Get-PartitionSupportedSize`, and its parsing is unit-tested only.
- **Not tested on a PC with BitLocker on, RST/VMD, an NVIDIA GPU, Wi-Fi, legacy BIOS, OneDrive
  redirection or an ARM CPU.** That logic is unit-tested with fixtures, but no real machine of those
  types was available. The BitLocker shell-property codes other than 2 ("off") come from documentation,
  not observation.
- winget column parsing assumes single-width characters. A program name in CJK/emoji would make
  that one row's columns shift. Registry data is unaffected.
- `.kdbx`/`.pst` files are only found inside the six library folders, not anywhere on disk.
- app_map.json: apt/dnf package names and some Flatpak IDs came from memory (mine and the seeding
  agent's) and have not been checked against live repositories. That matters from Stage 4
  (`restore.sh`), so verify them before generating install commands.
- Browser profile sizes include caches, so they overstate what needs migrating.

## Stage 2 — Distro chooser: DONE (2026-10-18)

```
target\release\linuxify.exe choose                 # asks 10 questions
target\release\linuxify.exe choose --answers tests\profiles\gamer.json
```

Output: `%LOCALAPPDATA%\Linuxify\choice.html` + `choice.json`. Uses the latest scan automatically
(`--no-scan` to ignore it).

### What works
- 10 plain-language questions; answers are combined with the scan (NVIDIA GPU, RAM, Steam library,
  Secure Boot, programs that won't run on Linux).
- Ranked list of all 11 distros, with a top 3 that each have "why it fits you" reasons and
  "good to know" cautions. Distros that need more RAM than the PC has are left out, with the reason shown.
- 14 new unit tests (54 total, all passing), including the six answer profiles in `tests/profiles/`.

### Verified
- **Six answer profiles**, run through the CLI on this PC's real scan (32 GB, AMD GPU, Secure Boot off):

  | Profile | #1 | #2 | #3 |
  |---|---|---|---|
  | newcomer (never used Linux, Windows look, hands-off) | Linux Mint 91% | Zorin OS 83% | Kubuntu 82% |
  | gamer (plays a lot) | Bazzite 86% | Pop!_OS 77% | Linux Mint 74% |
  | old_laptop (10+ yrs old) | Linux Mint 86% | MX Linux 81% | Zorin OS 79% |
  | developer (fresh, tinkerer, modern look) | Fedora KDE 85% | Fedora Workstation 78% | openSUSE TW 78% |
  | privacy_veteran (stable, terminal-comfortable) | Debian 89% | MX Linux 88% | Kubuntu 74% |
  | creator (pro creative work) | Pop!_OS 81% | Ubuntu 76% | Bazzite 75% |

- Unit tests also cover hardware this PC doesn't have: with a 3 GB-RAM scan the old_laptop profile
  puts MX Linux first and excludes Bazzite and Fedora. With an NVIDIA GPU, the gamer gets Bazzite
  then Pop!_OS, and the creator gets Pop!_OS. Secure Boot on lowers Pop!_OS's score and adds the
  warnings. Changing one answer (newcomer → heavy gamer) changes the top pick.
- Interactive prompt checked by piping keystrokes in: bad input is re-asked, `q` quits without saving,
  and typed answers give the same result as the matching profile file. Bug found and fixed: input
  piped from PowerShell starts with a byte-order mark, which made the first answer invalid.

### Not tested / known limits
- The trait ratings in `data/distros.json` are judgement calls, not benchmarks. Nobody has done an
  outside review yet.
- Not run with a real NVIDIA, low-RAM or Secure-Boot-on scan (unit tests only).
- English only.

### Update 2026-10-20 — 31 distros, 14 questions (requested by the owner)
- **Distros 11 → 31.** Added Linux Mint Xfce, Xubuntu, Lubuntu, Ubuntu MATE, elementary OS,
  Fedora Silverblue, Nobara, antiX, Linux Lite, Arch, EndeavourOS, Manjaro, CachyOS, Garuda,
  openSUSE Leap, NixOS, Void, Gentoo, Solus and AlmaLinux.
- **Questions 10 → 14:** wanting to learn Linux hands-on; how much software the package manager has;
  codecs/drivers out of the box vs free-software-only; and whether work uses an Ubuntu/Debian-based
  or a Red Hat-compatible Linux. Traits 14 → 20 (`diy`, `software_range`, `out_of_box`, `foss`,
  `enterprise`, `rhel_family`).
- **Profiles 6 → 10** (`tests/profiles/`). The tests now load these files directly. Results on this PC's real scan:

  | Profile | #1 | #2 | #3 |
  |---|---|---|---|
  | newcomer | Linux Mint 91% | Linux Mint Xfce 91% | Zorin OS 86% |
  | gamer | Bazzite 86% | Pop!_OS 81% | CachyOS 78% |
  | old_laptop | Linux Mint Xfce 94% | Linux Mint 87% | Linux Lite 86% |
  | developer | EndeavourOS 85% | CachyOS 83% | Garuda 82% |
  | privacy_veteran | Debian 87% | NixOS 86% | Arch 81% |
  | creator | Pop!_OS 83% | Bazzite 77% | Ubuntu 77% |
  | arch_tinkerer | Arch 94% | EndeavourOS 91% | NixOS 89% |
  | enterprise_worker (Ubuntu shop) | Ubuntu 85% | Fedora Silverblue 77% | Fedora Workstation 76% |
  | redhat_worker | AlmaLinux 75% | Fedora KDE 73% | Fedora Workstation 72% |
  | gaming_tinkerer | CachyOS 90% | Garuda 87% | EndeavourOS 83% |

- The tests check *properties*, not just names: e.g. every top-3 pick for the old laptop is
  lightweight, every pick for the enterprise worker has an enterprise rating ≥ 3, and on a 3 GB PC the
  heavy distros (Bazzite, Fedora, Garuda, elementary) are excluded. One test checks how many distros
  can't reach a top 5 under any test profile and hardware combination. Solus is the only one left that
  only shows up for unusual answer combinations (elementary, Leap and Tumbleweed were checked with
  realistic answer sets).
- 59 unit tests, all passing.
- Uncertain facts are marked rather than guessed: Secure Boot support is `unknown` for elementary,
  Nobara, antiX, Linux Lite and Solus. Arch-family distros, NixOS, Void and Gentoo are marked
  "needs Secure Boot turned off to install".

## Stage 3 — Try before installing: DONE (2026-10-20), VM trial only

```
target\release\linuxify.exe try                     # your top pick from `choose`
target\release\linuxify.exe try linux-mint          # downloads, verifies, boots in VirtualBox
target\release\linuxify.exe try fedora-kde --disk 25 --dry-run
target\release\linuxify.exe try zorin-os --iso-file D:\isos\Zorin-OS.iso
target\release\linuxify.exe try linux-mint --remove
target\release\linuxify.exe try --list
```

### What works
- Official ISO download for 15 distros (Mint, Mint Xfce, Ubuntu, Kubuntu, Xubuntu, Lubuntu,
  Fedora Workstation, Fedora KDE, Debian, Arch, Bazzite, openSUSE Tumbleweed, NixOS, AlmaLinux,
  Omarchy; Omarchy added 2026-10-22, pinned to 4.0.6),
  always picking the newest file in the official checksum list. Every one of the 14 was resolved live
  and HEAD-checked (all HTTP 200, 1.2-7.4 GB).
- SHA-256 verification before use, resumable downloads, a free-space check, and `--iso-file` for ISOs
  you already have (e.g. on the NAS), which must match the official hash.
- VirtualBox trial VM: UEFI, 4 GB RAM / 4 CPUs on this PC (sized from the scan), NAT network,
  no disk by default. `--dry-run` prints every command, and `--remove` deletes only Linuxify's own VM.
- 19 new unit tests (77 total, all passing; clippy clean).

### Verified end to end on this PC
1. Started the Linux Mint 22.3 download, killed it at 1.1 GB, and re-ran: it **resumed** from 1.1 GB,
   finished (2.88 GB at about 45 MB/s), and the SHA-256 matched Mint's official `sha256sum.txt`.
2. `linuxify try linux-mint` created the VM and booted the ISO under UEFI to GRUB, then to the **Linux
   Mint live desktop** with networking up (`docs/stage3-mint-live-in-virtualbox.png`).
3. `--remove` while the VM was running: powered it off, waited for VirtualBox to release it, and
   deleted the VM and its folder. The ISO was kept (confirmed through VirtualBox), and the two
   unrelated `<inaccessible>` VMs already registered in VirtualBox were untouched.
4. A fake ISO passed with `--iso-file` is refused with the official hash shown. Zorin (no automatic
   source) explains how to use `--iso-file`.
- Bugs found and fixed on the way: `--remove` failed because VirtualBox keeps a VM locked for a few
  seconds after power-off (now retried for up to 20 s); one line of main.rs had been mangled by a
  PowerShell encoding round trip.

### Not tested / known limits
- **Hyper-V and QEMU backends are not tested.** Hyper-V is installed here, but VM creation needs admin;
  QEMU isn't installed. Their command plans are unit-tested only.
- Only Linux Mint was booted for real. The other 13 ISOs were resolved and HEAD-checked, not downloaded.
- Checksum files are not GPG-verified (SHA-256 over HTTPS from the official host only).
- Pinned release folders (Fedora 44, Mint 22.3, Ubuntu-family 26.04, NixOS 26.05, AlmaLinux 10) will
  need bumping in `data/isos.json` when new releases come out. Debian, Arch, Tumbleweed and Bazzite
  follow "latest" automatically.
- The VM trial runs with Secure Boot off and on virtual hardware, so it can't show whether your real
  Wi-Fi/GPU work. A USB live stick is still the way to check that.
- Option B (boot from the real disk without a USB stick) was not built; see DECISIONS.md.

## Stage 4 — Data migration (staging): DONE (2026-10-20)

```
target\release\linuxify.exe migrate --target D:\ --dry-run     # totals, what's skipped and why
target\release\linuxify.exe migrate --target D:\               # copy + verify (re-run to resume)
target\release\linuxify.exe migrate --target D:\ --verify      # re-check an existing copy
target\release\linuxify.exe migrate --target E:\ --from-folder C:\Projects   # just one folder
```

### What works
- Copies your library folders, SSH keys, .gitconfig, fonts, VS Code settings, Minecraft saves, Prism
  instances, OBS scenes, Firefox/LibreWolf/Waterfox and Thunderbird profiles, and Chrome/Edge/Brave
  bookmarks into a Linux-shaped `linuxify-migration` folder, with `manifest.json`, `SHA256SUMS`,
  `restore.sh`, `apps.txt`, `README.txt` and `flagged.txt`.
- Resume after an interruption, SHA-256 verification, preserved timestamps, long paths, Unicode names,
  locked files (flagged, then picked up on the next run), and a free-space check.
- 13 new unit tests (90 total, all passing; clippy clean), including an interrupted-and-resumed
  copy, a locked file, a path over 260 characters, tamper detection, and generating apps.txt from
  the real app map.

### Verified
- **Test tree** (`scratch/make_fixture.py`, about 2,000 files, 734 MB, Unicode/emoji names, a path over
  260 characters, 1970 and 2009 timestamps, read-only/hidden files, junk files, a junction):
  - Dry run: 2,013 files / 734 MB planned, 5 items flagged with reasons, and **nothing written**.
  - Stopped after 700 files, then re-run: **700 reused, 1,313 copied, 2,013 verified**.
  - Hard-killed during a 200 MB file (leaving a 65 MB partial): the re-run replaced the partial, and
    Windows' own `Get-FileHash` confirms all three copies equal their sources.
  - Timestamps (incl. 1970/2009) identical; the junction wasn't followed; junk wasn't copied.
  - A deliberately edited copy was caught by `--verify`, and the next run re-copied it.
- **Your real profile, dry run only** (nothing copied): about 25,000 files, about 11 GB across 21 sources,
  36 shortcuts/system files flagged.
- **restore.sh in a real Linux Mint 22.3 VM** (fake profile generated by
  `scratch/make_fakeprofile.py`, migrated with the normal command, booted with `linuxify try`):
  **23 of 25 checks passed**; see `docs/stage4-vm-results.txt` and the screenshots in `docs/`.
  Passed: dry run changes nothing; all 22 checksums match; files restored with nesting and
  Unicode names; an existing file wasn't overwritten; timestamps preserved; ~/.ssh 700, private
  key 600, public key 644; warning about Windows paths in ssh config; Firefox and Thunderbird
  profiles added to profiles.ini; git installed via apt; a second run copies nothing and adds no
  duplicate profiles; `sha256sum -c` catches a damaged file.
  - The 2 failures were problems in the test setup, not Linuxify bugs: (1) the test script itself
    had a mangled "é" (I confirmed in the VM that `Résumé 2026.docx` was restored correctly);
    (2) the Audacity Flatpak needed more space than the live session's 2 GB RAM overlay. restore.sh
    reported that install as failed, carried on and exited cleanly, which is the intended behaviour.

### Not tested / known limits
- ~~A successful Flatpak install by restore.sh is not verified yet.~~ Verified in Stage 5 on an
  installed Mint: `com.microsoft.Edge` installed from Flathub.
- Only tested on Mint (apt). The dnf/zypper/pacman branches of restore.sh are untested.
- Browser profiles copied while the browser is running may be inconsistent. Locked files are flagged
  and picked up on re-run, but closing browsers first is best.
- Chromium-family passwords and cookies can't be migrated (Windows DPAPI); the README says how to
  export them.
- Folder names are kept in English (Documents, Pictures...). A non-English Linux install uses
  translated XDG folder names, so files would land beside them rather than in them.

## Stage 5 — Install: DONE (2026-10-21), tested in a throwaway VM only

Done and unit-tested (102 tests): install planner, preflight + `--replace-windows` gates, executor
with typed confirmation, `install-state.json` and `--undo`, the Linux-side `remove-windows.sh`
(dry by default, re-verifies restored files), and a `no-install.lock` that blocks all disk changes.
Checked on this PC **in dry-run only**: the plan arithmetic matches the real disk layout, and the
gates refuse correctly (not admin, staged copy on the Windows disk, a damaged staged file, the lock).

**This PC must never run a real install: the owner keeps Windows on it.** `no-install.lock` is set here.

Not tested yet: any real run (needs a throwaway Windows VM, which needs a Windows ISO and the
Windows licence accepted, the owner's call), and `remove-windows.sh` in a Linux VM (next).

### Stage 5 update (2026-10-21): real run in a throwaway Windows 10 VM
- Official Windows 10 22H2 ISO (SHA-256 matched Microsoft's published hash) installed in VirtualBox,
  UEFI with Secure Boot on, plus a second "data" disk. Never on the owner's PC.
- In the VM: scan (no blockers), ISO verified, migrate to E:, then **real `linuxify install` worked**:
  shrank C: by 89 GB, created and filled the LNXINST partition, added a one-time UEFI entry. **Restarting booted
  the Mint installer from LNXINST with Secure Boot on.** `--undo` also worked for real (C: back to 127.48 GB).
- Bugs found by the VM and fixed: exe needed the VC++ runtime (now static CRT); Windows' own
  `System Volume Information` failed the copy check; the installer offset wasn't MiB-aligned after an
  extend; and a crash could corrupt a resumed download (now 64 MB flushed checkpoints + one retry).
- **Big finding:** Ubuntu-family live systems (casper) auto-create an 85 GB "writable" persistence
  partition in the free space, and Ubiquity's "alongside" then offered to shrink the DATA disk instead.
  Fixed by adding `nopersistent` to casper GRUB entries on LNXINST.

### Stage 5 final run (2026-10-21, same VM, from the "before-install" snapshot)
Full path, Windows to Mint dual boot to Windows removed, all in the VM:
1. **Real install with the fix.** First attempt stopped at "could not update grub.cfg: Access is
   denied": files copied off an ISO keep the read-only attribute. Fixed (attribute cleared first).
   `--undo` from that half-finished state worked. The next run was refused because the plan had
   changed (91 to 89 GB) and the old typed phrase no longer matched, as intended. Then it worked.
2. **Live session: no "writable" partition**, `nopersistent` on the kernel line, 91 GB free on sda.
3. **New finding: Ubiquity stopped with "Failed to unmount partitions: /cdrom".** The live system keeps
   LNXINST mounted, and it's on the disk being repartitioned. Fixed by also adding `toram` (live
   medium copied to RAM). Re-tested from a fresh install run: `/cdrom` is tmpfs, nothing on sda mounted,
   and **"Install Linux Mint alongside Windows Boot Manager" formatted a new partition #6 on sda,
   not the data disk** (`docs/stage5-live-toram.png`, `docs/stage5-ubiquity-alongside.png`).
4. **Mint installed and booted** (shim + Secure Boot on). Firmware menu: Ubuntu, Windows Boot Manager,
   and Linuxify's installer entry. Windows had booted normally before (one-time entry consumed).
5. **restore.sh on the installed system:** dry run changed nothing; real run: 7/7 checksums, 7 files
   restored, `.ssh` 700 / key 600 / pub 644, timestamps kept, **Flatpak Edge installed**, apt package
   installed; a second run copied nothing.
6. **remove-windows.sh:** check-only listed exactly WINRE, MSR, Windows and LNXINST plus 2 boot entries.
   It **refused** when a restored file was edited and when one was missing, and passed again once the
   file was put back. `--really` with the typed phrase deleted those 4 partitions and both boot entries
   (`docs/stage5-remove-windows-plan.png`, `-done.png`). After a restart: sda holds only EFI + Linux,
   Linux boots, the data disk (E:) is untouched and its staged copy still verifies.
   - **Bug found and fixed:** `EFI/Microsoft` was not renamed, because `/boot/efi` is mounted root-only
     and the check ran without sudo. GRUB then still listed a dead Windows entry. Fixed (`sudo test -d`);
     the fixed block was run in the same VM: renamed, and `update-grub` dropped the Windows entry.
   - Also fixed: the Linuxify installer's own UEFI entry was left behind (now removed too); sfdisk's
     harmless "re-reading failed" noise is hidden; the script now says to restart once.

### Stage 5 per-distro run (2026-10-21/22): every downloadable ISO, two throwaway VMs
Each ISO went through dry run, real install with `--linux-gb 80` (70 for Bazzite), restart into the
installer from LNXINST, then the distro's own installer, all in the VMs (headless, driven through the
VirtualBox API). Log: `scratch/distro-tests/results.txt`.

| Distro | Installer | Result |
|---|---|---|
| Linux Mint 22.3 Cinnamon / Xfce | Ubiquity | PASS (toram), installed, boots |
| Ubuntu 26.04.1 | Flutter installer | PASS, installed, GRUB lists Windows |
| Kubuntu 26.04.1 | Calamares | PASS (2026-10-22, new LNXDATA partition for its 4.08 GiB squashfs): runs from NTFS, installed, both OSes boot, `--undo` removes both partitions |
| Xubuntu / Lubuntu 26.04.1 | Flutter / Calamares | PASS after fixes (toram only for Ubiquity; Calamares hint), installed; `--undo` after install verified |
| Debian 13 | Calamares | PASS, installed, firmware lists Debian + Windows |
| Fedora 44 KDE / Workstation | Anaconda (live) | PASS after fix: Anaconda hid the Windows disk; now `rd.live.ram=1` + release hook. Both installed, ESP intact, GRUB lists Windows (hidden menu on Workstation: hint says Esc), `--undo` OK |
| openSUSE Tumbleweed | YaST | PASS after fixes: kiwi needs `CDLABEL=`; YaST hid the Windows disk and offered to shrink the backup drive, now kiwi RAM hook; hint says pick GRUB2 (systemd-boot overfills Windows' 100 MB ESP) |
| Bazzite | Anaconda (installer ISO) | PASS: installed from LNXINST, MOK enrolled, boots |
| AlmaLinux 10.2 | Anaconda (live) | Can't boot in this VM (CPU is x86-64-v2, Alma 10 needs v3); new CPU-level gate refuses it on such PCs |
| Arch | manual | PASS: boots from LNXINST (systemd-boot entries adapted); installed by hand (pacstrap, GRUB on Windows' ESP at /boot/efi), both OSes boot, `--undo` OK |
| Omarchy 4.0.6 | Omarchy (archinstall) | PARTIAL (2026-10-22): LNXINST + LNXDATA created, live system boots and runs from LNXDATA (ntfs3). First run showed its installer hides the Windows disk; fixed with `copytoram=y` (needs ~9 GB RAM). The full install with that fix is not yet confirmed |
| NixOS | — | Refused as designed: its initrd only mounts an ISO 9660 filesystem with the ISO's label |

### Not tested / known limits
- Full installs to the end were done for every offered distro except AlmaLinux (refused) and
  NixOS (refused). AlmaLinux shares Fedora's path but was never booted (see the CPU note); on a
  v3 CPU it is untested (needs a VM backend that exposes AVX2, or real hardware).
- Low RAM (2026-10-22): with 4 GB the plan refuses Mint (needs 5.9 GB) and Fedora Workstation
  (5.3 GB) before any change; with 5.4 GB, just over Fedora Workstation's line, its live system
  ran from RAM and the install completed. Ubiquity's `toram` was tested at 6 GB only.
- LNXDATA (NTFS) was tested to the end with Kubuntu; Omarchy boots and runs from it, but its
  install (with `copytoram`) and `--undo` are not yet confirmed. Ubuntu-family ISOs that later
  grow past 4 GB per file should work the same way, but each new one is untested until tried.
- The fixed remove-windows.sh rename step was checked as an extracted block, not with a second full
  `--really` run from scratch.
- Not tested on real hardware, BitLocker-encrypted disks (refused by design), or disks where C: isn't
  followed by free space Windows can shrink into.
- Apps that need a kernel module (the VirtualBox apt package) install but don't load with Secure
  Boot on until the module is signed/enrolled. That's the distro's MOK flow, not Linuxify.
- The "Résumé" file name in the VM test data was mangled by the test-setup script (PowerShell 5.1
  read it as ANSI); restore.sh kept the bytes exactly, as before.

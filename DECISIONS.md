# Decisions

Short record of choices made while building Linuxify, and why.

## Stage 1 — scanner

- **Hardware facts come from a read-only PowerShell/CIM script** (`src/scan/hardware_probe.ps1`),
  embedded in the exe and run with `-EncodedCommand`. It only calls `Get-*` cmdlets and CIM queries,
  and each section is wrapped in try/catch. A failed section is reported as "unknown" and does not
  crash the scan. Rust only parses the JSON it prints, and that parsing is unit-tested. I chose this over
  calling WMI through `windows-rs` COM, which would need a lot of `unsafe` code for no benefit.
- **Boot facts that need no admin rights come from the registry**: Secure Boot
  (`SecureBoot\State\UEFISecureBootEnabled`) and Fast Startup (`HiberbootEnabled`).
  UEFI vs BIOS comes from `PEFirmwareType`, with the `firmware_type` environment variable
  Windows sets as a fallback. The fallback was needed because this PC has no `PEFirmwareType` value.
  `GetFirmwareType()` would need `unsafe` FFI, so I didn't use it.
- **BitLocker without admin**: I use the Explorer shell property `System.Volume.BitLockerProtection`
  (2 = off, any other known code = encrypted in some form). When the scan runs as admin,
  `Get-BitLockerVolume` takes priority. A volume that is "suspended" still counts as encrypted.
- **Exact shrinkable size** (`Get-PartitionSupportedSize`) needs admin. Without admin I fall back to
  "free space ≥ 50 GB", which is reported as *info* ("not verified"), never as a pass.
  Stage 5 must re-check this as admin.
- **Blockers (safety rule 4)** each get a stable id, which later stages must check:
  `bitlocker-active`, `bitlocker-unknown`, `rst-vmd`, `low-space`, `space-unknown`,
  `secure-boot-unknown`, `firmware-unknown`, `hardware-probe-failed`, `arm-cpu`.
  An "unknown" is always treated as a blocker, not a pass.
- **Programs are merged by exact normalised name** (lowercase, ™/®/© and control characters removed,
  whitespace collapsed). Version numbers are deliberately *not* stripped: "Portal" and "Portal 2"
  are different programs. Real registry names contained NUL padding and raw cp1252 `0x99` (™) bytes,
  which is why control characters are removed.
- **winget** is run with `--disable-interactivity`, so Linuxify never accepts a source agreement
  for the user. If winget is missing or prompts, the scan continues with registry data only.
- **Microsoft-signed Store packages** (publisher hash `8wekyb3d8bbwe`) that aren't in the map are
  grouped as "built-in Windows components" rather than listed as unknown programs. Calculator,
  Photos and similar would otherwise flood the report. Ones that *are* in the map, such as Fortnite
  or Minecraft, still show normally.
- **app_map.json gets a sixth bucket, `ignore`**, for runtimes and drivers (VC++ redistributables,
  .NET runtimes, WebView2, chipset drivers). Reporting these as "unknown" would be noise. They appear
  in a collapsed list in the report.
- **Matching uses `contains` / `^prefix` / `=exact` patterns, not regex**, and the longest pattern wins.
  This is easier to edit by hand, and it removes the regex dependency. Format: `data/SCHEMA.md`.
- **app_map.json is compiled into the exe** as a fallback. A `data/app_map.json` next to the exe, or
  `--data-dir`, overrides it, so the list can be edited without rebuilding.
- **Steam games are detected from the registry key `Steam App <id>`**. Every Steam game gets a
  ProtonDB link built from its real app id, so unmapped games stay honest (`unknown`) but still
  point somewhere useful.
- **Folder walking never follows junctions/symlinks** (Windows profiles contain junction loops),
  and it only reads metadata, so OneDrive cloud-only files are not downloaded. They are counted
  separately because they must be downloaded before migration.
- **Reports are written only under `%LOCALAPPDATA%\Linuxify\reports\`**: a timestamped folder plus
  `latest/`. There is no `--out` option, because of safety rule 1. The HTML has no scripts and no external
  requests, escapes every scanned string, and only http(s) links become clickable.
- **app_map.json was seeded by a Haiku subagent, then reviewed by hand.** I corrected: Satisfactory
  (was "native", now wine_ok), PuTTY (has a Linux build), Beyond Compare (has Linux builds),
  Minecraft Launcher (official Linux launcher for Java Edition), Unity Editor (officially supported),
  MCA Selector (Java), Supersonic (cross-platform), GitHub Desktop (community build only, so
  "alternative"), Skype (shut down in May 2025), plus pattern fixes (Deadlock was catching
  "Deadlock Mod Manager"; Java(TM), Unity 6, Outlook for Windows, whkd weren't matching).

## Stage 2 — distro chooser

- **distrochooser.de is not used directly.** It has no public API, and its About page
  (https://distrochooser.de/info/about/en) says the question/answer structure and result matrix are
  proprietary and may only be used with prior permission. Linuxify uses its own built-in quiz and only
  *links* to distrochooser.de as an optional second opinion (no embedding, no scraping).
- **Scoring is a transparent weighted sum.** Every answer adds weight to traits (beginner, gaming,
  nvidia, light, stable, fresh, windows_like, modern_look, privacy, low_maintenance, customizable,
  dev, community, creative). Every distro rates itself 0–5 per trait in `data/distros.json`.
  Score = Σ weight × rating ÷ (5 × Σ weight), shown as a percentage. Reasons = the distro's
  highlight text for its three biggest contributing traits. "Weaker on X" is added when the user
  weighted X ≥ 2 and the distro rates it ≤ 2.
- **The quiz questions and their weights live in code (`src/chooser.rs`), not JSON**, so they are
  type-checked and covered by tests. The distro ratings are data and easy to edit.
- **Scan input**: an NVIDIA GPU adds +3 nvidia weight; RAM < 4 GB adds +4 light, < 8 GB adds +1.5;
  ≥ 5 Steam games adds +1 gaming, but only if the user didn't answer "rarely or never" (the user's
  answer wins). A distro whose `min_ram_gb` exceeds the PC's RAM (+0.5 GB slack) is excluded and
  shown as "not suitable". With Secure Boot on, a distro that needs it disabled loses 5 points and
  gets a caution. Distros needing key enrolment, or whose support is unknown, get a caution only.
- **Distro set (11)**: Linux Mint, Ubuntu, Kubuntu, Fedora Workstation, Fedora KDE, Pop!_OS,
  Bazzite, Debian, openSUSE Tumbleweed, Zorin OS Core, MX Linux. Ratings are my judgement, not
  measurements. Two uncertain facts are recorded as such: Pop!_OS is marked "needs Secure Boot
  disabled" (historically true; the caveat says to check the current release), and MX Linux's Secure
  Boot support is "unknown". No version numbers are stored, so the file doesn't go stale. ISO URLs and
  checksums are added in Stage 3.
- **Output**: `%LOCALAPPDATA%\Linuxify\choice.json` (answers + full ranking; Stage 3 will default to
  `top[0]`) and `choice.html`. `--answers` accepts either a plain answers file or a saved choice.json.

### 2026-10-20: expanded to 31 distros / 14 questions
- I picked the list as "distros with a sizeable desktop user base". I left out special-purpose
  systems (Tails, Kali, Qubes), SteamOS (no official general-PC installer) and very new projects
  (KDE Linux).
- The "work" question distinguishes Ubuntu/Debian shops from Red Hat-compatible shops, backed by a
  `rhel_family` trait. Without it, AlmaLinux lost to Debian even for Red Hat users.
- Answering "less than 5 years old" adds a small (0.5) preference for a modern desktop, so a newer PC
  gets Mint Cinnamon ahead of Mint Xfce instead of a tie.
- openSUSE Tumbleweed's stability went 3→4 and maintenance 2→3. It is the most heavily tested
  rolling release (openQA plus automatic snapshots), and rating it like Arch was unfair.

## Stage 3 — try before installing

- **Only Option A (VM trial) ships. Option B (boot-from-disk trial) was deliberately not built.**
  A boot-from-disk trial means writing to the real EFI System Partition and the firmware boot
  entries. That is the same risk class as Stage 5's install, but without Stage 5's gates. It also
  interacts badly with the things Stage 1 flags: changing boot entries triggers BitLocker recovery,
  and with Secure Boot on, GRUB-loopback/Ventoy-style booting needs a one-time key enrolment (MOK)
  at a firmware-level screen that Linuxify can't drive or undo. On this PC the EFI partition is even
  on a different disk from Windows. The VM trial already answers "will I like it?" with zero risk.
  For "does my real hardware work?", the standard answer is a USB live stick, and Stage 5 will handle
  real boot entries with dry-run, typed confirmation and undo. So `--undo` isn't needed; `--remove`
  deletes the trial VM.
- **ISO sources live in `data/isos.json`**: an official checksum URL, the hosts it may come from,
  and a name selector (prefix/contains/suffix). Linuxify fetches the checksum list, rejects it if any
  redirect leaves the allowed hosts, picks the newest matching file (natural version sort), and
  downloads it from the same official folder. Fedora is the one exception: the ISO comes through
  Fedora's own mirror redirector (`download_base`), because Fedora asks people not to download from
  its master server, but the checksum still comes from dl.fedoraproject.org. 14 of the 31 distros are
  automatic. The rest say where to download and accept `--iso-file`, which is still verified when an
  official checksum source exists, and loudly unverified when it doesn't.
- **Verification is SHA-256 against the official list fetched over HTTPS** (`https_only`, rustls).
  GPG signature checking of the checksum files is *not* implemented yet. It would add protection
  against a compromised distro web server. Noted as a known limit.
- **Downloads resume** with an HTTP Range request into `<iso>.part`, check free space first, and are
  renamed to the final name only after the SHA-256 matches. A mismatch deletes our own `.part` file.
  A `<iso>.verified` marker (hash + size) avoids re-hashing 3-7 GB on every run; `--reverify` forces it.
- **VM backends**: VirtualBox (preferred, tested), Hyper-V (Gen 2 with the
  `MicrosoftUEFICertificateAuthority` Secure Boot template; needs admin, untested), QEMU (WHPX
  acceleration with TCG fallback; untested because it isn't installed and its installer needs admin).
  VMs get UEFI firmware, NAT networking and **no virtual disk unless `--disk N` is given**, so the
  default trial can't write anywhere. Every VM is named `Linuxify trial - <distro>`, and only VMs
  with that prefix are ever touched. `--remove` ejects the ISO *before* `unregistervm --delete`,
  so VirtualBox can never delete an ISO, even one passed with `--iso-file`.
- **Linuxify won't auto-download VM software or other executables.** It prints the winget command
  instead (running downloaded installers is a step the user should choose).
- **Dev-environment note (not a product issue):** the Claude desktop app is an MSIX package, so files
  its tools write under `%LOCALAPPDATA%` are silently redirected to
  `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Local\`. VirtualBox runs outside the package and can't
  see them (VERR_PATH_NOT_FOUND). For testing, `scratch/run-outside.cmd` is launched via explorer.exe
  to run Linuxify as a normal process. Real users running linuxify.exe from a terminal are unaffected.

## Stage 4 — data migration (staging)

- **Copy, never move.** Sources are opened read-only. Everything is written inside
  `<target>\linuxify-migration\`. The target must not overlap any source (either way round) and
  must not be a system folder. Sources nested in other sources are dropped so nothing is copied twice.
- **Staging layout mirrors the Linux home folder** (`home/Documents`, `home/.ssh`,
  `home/.config/Code/User`, `home/.local/share/fonts`, `home/.minecraft`...), so restoring is one copy.
  English folder names are kept, since they match the XDG defaults on every distro set to English.
  Firefox-family and Thunderbird profiles are copied as whole profile folders minus caches and lock
  files; they're cross-platform. **Chromium-family browsers: only `Bookmarks` is copied.** Saved
  passwords and cookies are encrypted with Windows DPAPI and are unreadable on Linux, so the README
  tells the user to export passwords or use browser sync. Minecraft copies only saves, packs,
  screenshots and options (not the Windows game files); VS Code copies settings, keybindings,
  snippets and tasks (not caches).
- **Skipped and flagged rather than guessed:** shortcuts (.lnk), desktop.ini/Thumbs.db, Office
  `~$` lock files, junctions/symlinks (never followed), OneDrive online-only files (copying would
  silently download them), names longer than Linux's 255-byte limit, and files locked or changed
  during the copy. All are listed in `flagged.txt` with the reason.
- **Integrity:** each file is hashed (SHA-256) while it's copied, written to `<name>.linuxify-partial`,
  and renamed only when complete. Modified, accessed and created times are preserved. After copying,
  every file is re-read and checked against `manifest.json` (skip with `--no-verify`).
  `SHA256SUMS` is in `sha256sum -c` format, so the Linux side can verify with standard tools.
- **Resume:** an append-only `.linuxify-progress.jsonl` records each finished file. A rerun reuses
  a file only if the source size and mtime are unchanged and the copy is still present at the right
  size; anything else is copied again. A truncated last log line (from a crash) is ignored.
- **`restore.sh` is static** (not templated), so it can be reviewed once and trusted. It reads
  `apps.txt` and `SHA256SUMS` at runtime. It never overwrites an existing file in `$HOME`, adds
  Mozilla profiles as *extra* profiles (never replacing the default), supports `--dry-run`, and asks
  before installing apps. Apps install from Flathub per-user (works on every distro); native packages
  use whichever of apt/dnf/zypper/pacman exists. Failed installs are listed at the end instead of
  aborting the run.
- **apps.txt** is generated from the scan: for each mapped program, the native app's Flatpak, else
  the first alternative that has a Flatpak, else native package names. Blocked and unknown programs
  are left out; nothing is invented. All 49 Flatpak IDs in app_map.json were checked against the
  Flathub API before this was built (one was wrong, `org.lmms.LMMS` → `io.lmms.LMMS`, and is fixed).
- `%LOCALAPPDATA%\Linuxify\migration.json` records where the staged copy is. Stage 5's "remove
  Windows" gate will need it.
- `--target` takes a folder (another drive, USB disk or NAS share). Linuxify never formats or
  partitions anything for this stage. FAT32 targets are refused when a file is 4 GB or larger.

## Stage 5 — install (dual boot)

- **No pre-seeded/automatic installer.** Linuxify prepares the disk and boots the distro's own
  installer; the user still clicks through it. An unattended install that partitions a real
  disk is too risky to hide behind one confirmation, and each distro's autoinstall format differs.
- **Shrink C:, then put the installer on a small FAT32 partition ("LNXINST") right after it.**
  UEFI firmware boots `EFI\BOOT\BOOTX64.EFI` from FAT32 directly, so no USB stick, no GRUB
  chain-loading and no Ventoy. The shim is the distro's own signed one, so Secure Boot can stay on.
  The rest is left *unallocated* for the installer's "install alongside" option.
- **One-time boot entry only** (`bcdedit /copy {bootmgr}` + `{fwbootmgr} bootsequence`): the next
  restart starts the installer; later restarts go back to Windows by themselves.
- **Typed confirmation that names the disk and size** (`SHRINK C: ON DISK 0 BY 89 GB`). The phrase
  is recomputed on every run, so an old phrase can't approve a new plan (seen in the VM: after an
  undo the plan changed from 91 to 89 GB and the old phrase was refused).
- **`install-state.json` is written before the first change** and after every step, so
  `--undo` works from any point, including a half-finished install (verified in the VM after a failure
  in the copy step).
- **`no-install.lock`** in `%LOCALAPPDATA%\Linuxify` blocks every disk change on a machine that
  must never be converted (the developer's own PC).
- **Removing Windows happens from Linux, not Windows** (`remove-windows.sh`). It refuses on a
  live session, re-checks every restored file in `$HOME` against SHA256SUMS (count + hash), and
  deletes only the Windows, MSR, recovery and LNXINST partitions on the Windows disk. EFI is kept;
  `EFI/Microsoft` is renamed, not deleted; Windows' and Linuxify's boot entries are removed.
- **Static C runtime** (`.cargo/config.toml`, `+crt-static`): a fresh Windows has no VC++
  redistributable, and the exe failed to start in the VM without it.

### Ubuntu-family (casper) live systems on LNXINST — found in the VM test
- **`nopersistent`**: casper creates an 85 GB "writable" persistence partition in any free space on
  the disk it booted from, i.e. exactly the space meant for Linux. Ubiquity's "alongside" then
  offered to shrink the *data disk* instead.
- **`toram` only for Ubiquity (Mint), and only with enough RAM.** Ubiquity stops with "Failed to
  unmount partitions: /cdrom" while LNXINST is mounted from the disk it repartitions; `toram` copies
  the live medium into RAM and releases it. It is NOT a general fix, all found in the VM:
  Xubuntu's 5.2 GB ISO copied into 6 GB of RAM hung before the desktop appeared; Lubuntu's Calamares
  failed in `unpackfs` ("/dev/loop1 already mounted") because casper had already loop-mounted the
  RAM copy; Ubuntu's and Xubuntu's Flutter installer and Lubuntu's Calamares all install fine with
  LNXINST still mounted. So `data/distros.json` records each distro's `installer`, and the plan adds
  `toram` only for `ubiquity`, refusing (use a USB stick) when RAM < ISO size + 3 GB.
- `nopersistent` is added right after the kernel path of every casper line (`/casper/` in the path
  or `boot=casper`, so it lands before `---`). Ubuntu 26.04's lines have no `boot=casper` at all;
  matching only that missed them, and live Ubuntu made a 77 GB "writable" partition.
- **Installer-specific instructions** (`installer_hint`). Ubiquity and the Flutter installer say
  "Install … alongside Windows Boot Manager" and use the free space. In Calamares, "Install
  alongside" *shrinks Windows again*; the right choice is "Replace a partition" + the Free Space
  block. Anaconda: "Share disk with other operating systems", "Reclaim additional space" unticked
  (its review lists the ESP as "format as efi", but it only mounts it: the Windows files were
  still there and Fedora's GRUB lists Windows). Unknown installers get a generic "install into the
  free space, never Erase disk". Every hint starts by naming the Windows disk by size (GiB and GB,
  since installers differ): with a second drive in the VM, Calamares (Debian) and Anaconda both
  preselected the *other* drive.
- **Anaconda hides the disk it booted from.** It resolves the live device (`/run/initramfs/livedev`
  → LNXINST) and protects the whole disk plus every sibling partition, so the Windows disk vanished
  from Fedora's destination list and only the backup drive was offered. Fix, VM-verified by hand
  first: `rd.live.ram=1` (dracut copies the image to RAM), then unmount `/run/initramfs/live` and
  delete `livedev` before the installer starts. `adapt_live_boot` adds `rd.live.ram=1` (dropping
  `rd.live.check`) and a second initrd, `linuxify\release.img`, to every `initrd` line; it is a
  one-file newc cpio (`release_initrd`) holding a dracut pre-pivot hook that does the unmount.
  It has no directory entries on purpose: in Fedora's and AlmaLinux's initrds `lib` and
  `usr/lib/dracut/hooks` are symlinks, and a directory entry would replace them. The files go to
  `var/lib/dracut/hooks/…`, the real hook directory in Fedora, AlmaLinux and openSUSE alike
  (openSUSE has no `usr/lib/dracut/hooks` at all). The RAM rule (ISO + 3 GB) applies to Anaconda
  as to Ubiquity, counting only the squashfs that dracut copies (the ISO's largest file) instead
  of the whole ISO.
- **YaST skips every disk with a mounted filesystem** (`DiskAnalyzer#available_device?`), and
  openSUSE's kiwi live system loop-mounts `LiveOS/squashfs.img` straight off LNXINST, so again
  only the backup drive was offered, and YaST's default plan *shrank its NTFS to 1 GB*. kiwi has
  no copy-to-RAM for local media, so `release.img` carries a second hook (pre-mount, before the
  root is assembled) that copies the squashfs into kiwi's own tmpfs, remounts both squashfs
  mounts from the copy and unmounts LNXINST; any failed step leaves things as they were. VM
  result: YaST's default plan then used the Windows disk's free space (new root + swap, ESP
  mounted, not formatted) and touched nothing else. openSUSE is `"installer": "yast"`, which runs
  from RAM with the same squashfs-only RAM rule.
- **openSUSE: tell the user to pick GRUB2.** YaST's default boot loader is now systemd-boot, which
  keeps kernel and initrd on the ESP; Windows' 100 MB ESP was 91% full after one kernel, so the
  next update wouldn't fit. With "GRUB2 for EFI" (Booting on the last screen) the ESP stayed at
  34% and GRUB listed Windows. Linuxify can't choose this for the user, so the YaST hint says it.
- **Installer ISOs (Bazzite) need no RAM copy.** Anaconda booted with `inst.stage2=hd:` is the
  install-from-hard-drive mode: it protects only the source partition, and the Windows disk was
  offered with its free space. It also reads ~4.6 GB of ostree data from LNXINST during the
  install, so a RAM copy would be impossible anyway. Own installer type, `anaconda_classic`, with
  a hint matching its older screen ("Automatic", "Free up space…" unticked).
- **Some ISOs can't start from a partition at all.** NixOS's initrd mounts
  `/dev/disk/by-label/<ISO label>` as `iso9660`; a FAT32 partition can be neither, and its systemd
  initrd has no dracut hooks to patch. `usb_only` in `data/distros.json` carries the reason, and
  the plan refuses with "use a USB stick" before any change.
- **systemd-boot entries are adapted too.** Arch's ISO boots with systemd-boot on UEFI
  (`loader\entries\*.conf`) and finds its medium by ISO UUID (`archisosearchuuid=`); that becomes
  `archisolabel=LNXINST`.
- **CPU level gate.** AlmaLinux 10 is built for x86-64-v3 (AVX2) and stops right after the
  kernel with "CPU does not support x86-64-v3" on older CPUs (seen in the VM, whose virtual CPU is
  v2). The scan now records the CPU's level from CPUID (`x86_64_level`); a distro with a higher
  `min_cpu_level` is left out by the chooser and refused by the install plan, before any change.
- **Keep `CDLABEL=` as `CDLABEL=`.** openSUSE's live root comes from kiwi's dracut generator,
  which only accepts `root=live:CDLABEL=…`; rewriting it to `LABEL=` dropped the VM into the dracut
  emergency shell. dracut's own dmsquash accepts both, so only the label value is replaced.
- **FAT32 ignores case.** Fedora 44's ISO has both `EFI/BOOT/BOOTX64.EFI` and `bootx64.efi`
  (identical), so 340 ISO files become 338 on LNXINST and the post-copy check stopped the install.
  The ISO is now counted the way FAT32 will hold it (`fat_view`); case-twins with different
  contents (hashed) are refused before any disk change, "use a USB stick".
- **`--undo` after Linux is installed is a safe cleanup**: it deletes only LNXINST (found by exact
  offset and size) and its boot entry and extends C: into the freed 5 GB; the Linux partition and
  its boot entry stay (VM-verified with Lubuntu). The end-of-install message says so.
- **One adapter for every live-boot family** (`adapt_live_boot`, unit-tested with real lines from
  each ISO, idempotent), applied to `boot\grub\grub.cfg`, `loopback.cfg`, `boot\grub2\grub.cfg` and
  `EFI\BOOT\grub.cfg`: dracut's `root=live:CDLABEL=…` and Anaconda's `inst.stage2=hd:LABEL=…`
  point at LNXINST (label value only, see above) (the ISO's own label no longer exists once the files are on a partition);
  GRUB's `search -l '<iso label>'` is changed the same way; Debian's `boot=live` gets `toram` only via the rule above;
  openSUSE's `rd.live.overlay.persistent` is removed for the same reason as `nopersistent`.
  The ISO's files keep their read-only attribute after copying, so it is cleared first.
- **Omarchy (archiso) uses LNXDATA too, and runs from RAM.** Omarchy 4.0.6's `airootfs.sfs` is
  6.1 GB. archiso finds its medium by label, so its boot line gets `archisolabel=LNXDATA` (not
  LNXINST); its initramfs has the kernel's `ntfs3` and `ntfs-3g`. VM result: live
  `/run/archiso/bootmnt` = LNXDATA (ntfs3). archiso's GRUB finds its config by a
  `/boot/<uuid>.uuid` file that's on both partitions, so the configs on LNXDATA are patched the
  same way. Omarchy's installer leaves out the disk `/run/archiso/bootmnt` is on (the Windows
  disk), so it gets `copytoram=y` (archiso copies the image to RAM, then unmounts the medium)
  and the RAM check (image + 3 GB, about 9 GB). Omarchy's free-space install makes its own ESP
  and never touches Windows', and Windows is added to its Limine menu with `limine-scan`.
- **A file over 4 GB can't go on FAT32, so casper ISOs get a second, NTFS partition ("LNXDATA").**
  Kubuntu 26.04's squashfs is 4.08 GiB. The firmware can only boot from FAT32, but casper doesn't
  care where its squashfs is: it mounts every partition (`is_supported_fs` includes `ntfs`, and
  Ubuntu's initrd ships `ntfs-3g`) and uses the first with `casper/*.squashfs`. So LNXINST gets
  every file except the big ones (`robocopy /MAX:`) and LNXDATA, right after it, a full copy of
  the ISO; casper skips LNXINST (no squashfs) and runs from LNXDATA. NTFS rather than exFAT or UDF
  because Windows formats it natively and Kubuntu's initrd has `ntfs-3g` but no exFAT module.
  LNXDATA is part of `installer_size`, so shrink, undo and the "after install" cleanup cover both
  (undo deletes the higher partition number first). VM result: live `/cdrom` = LNXDATA (fuseblk),
  Calamares installed into the free space, Kubuntu and Windows both boot, `--undo` gave C: back
  all 10 GB. Other live systems find their medium by one label, so a big file there is still
  refused ("use a USB stick"). The dry run mounts the ISO read-only to find big files, so this
  shows up before the confirmation, not after.
- **Fedora hides its boot menu**, even with Windows next to it. The Anaconda hint says how to get
  to Windows (Esc, then `normal` at the `grub>` prompt).
- **Hand-made boot loaders (Arch): Windows' ESP at `/boot/efi`, not `/boot`.** At 100 MB it can't
  hold kernels. VM-verified: GRUB on the ESP used 28%, os-prober found Windows.
- **Secure Boot gate.** With Secure Boot on, a distro marked `needs_disabling` or `unknown` in
  `data/distros.json` is refused before any disk change: the firmware would refuse the installer,
  leaving C: shrunk and nothing to boot. `needs_key_enrolment` (Bazzite) is allowed; its own
  installer guides the enrolment.
- **Battery gate.** On a laptop running on battery (`Win32_Battery` status discharging/low/critical)
  the install refuses until the charger is plugged in; the scan shows it as a warning.

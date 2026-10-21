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
- Official ISO download for 14 distros (Mint, Mint Xfce, Ubuntu, Kubuntu, Xubuntu, Lubuntu,
  Fedora Workstation, Fedora KDE, Debian, Arch, Bazzite, openSUSE Tumbleweed, NixOS, AlmaLinux),
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

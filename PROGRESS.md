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

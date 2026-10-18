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

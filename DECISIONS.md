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

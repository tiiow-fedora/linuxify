# Linuxify — build handoff

You are building **Linuxify**: a native Windows program that helps a user migrate from Windows to Linux. It scans the machine, recommends a distro, lets the user trial it safely, stages their data, and (last, and only with explicit confirmation) installs Linux. The owner is not a systems programmer and wants this *built*, not discussed. Make decisions, note them in `DECISIONS.md`, keep moving. Ask only when something is genuinely his call.

Project folder: `%USERPROFILE%\linuxify\` (create the cargo project here, `git init` it).

## Language and stack (decided)

- **Rust**, stable toolchain, MSVC target. Reason: native single `.exe`, no runtime to install, memory-safe (this tool touches disks, so no C-style footguns), good Windows bindings. The owner does not know Rust and does not need to.
- Crates to start with: `windows` (windows-rs) or `winreg` for the registry, `serde` + `serde_json` for data tables and reports, `clap` for the CLI, `reqwest` (blocking, rustls) for downloads, `sha2` for ISO verification. Add others only if needed.
- Stage 1–2 are a **CLI** that also writes a self-contained HTML report. A GUI (egui) is a later, optional stage; do not start it before the CLI stages work.
- Install Rust via rustup if missing (ask before installing anything system-wide).

## Hard safety rules (non-negotiable)

1. Stages 1–4 are **read-only on the user's real data and disks**. They may create files only under `%LOCALAPPDATA%\Linuxify\` and copy user data to a staging target the user picks.
2. **Never format, delete, repartition, or modify the boot configuration** without an explicit typed confirmation naming the exact disk/partition and a verified backup check. Default install mode is **dual boot** (shrink C:, keep Windows). "Remove Windows" is a separate option that only unlocks after migrated data is verified (file count + hash check) on the Linux side.
3. Every destructive step must support `--dry-run` that prints exactly what it would do. Build dry-run first, real run second.
4. Detect and refuse to proceed when these are unhandled: BitLocker active on the target volume (tell the user how to suspend/decrypt), Intel RST/VMD NVMe that hides the disk from Linux, less than ~50 GB free to shrink, Secure Boot state unknown. Report them in the readiness report instead of failing late.
5. Verify every downloaded ISO against the distro's published SHA-256 before using it. Download only from the distro's official mirrors; hardcode the URLs/checksum sources in a data file.

## Stages (each must be useful on its own; finish and test one before the next)

### Stage 1 — Scanner + Linux Readiness Report
`linuxify scan` →
- **Installed programs**: registry uninstall keys (HKLM 64/32-bit, HKCU) plus `winget list`; dedupe; capture name, publisher, version.
- **Hardware/boot facts**: CPU, RAM, GPU vendor (NVIDIA/AMD/Intel), disks + partition layout, UEFI vs BIOS, Secure Boot state, BitLocker status, NVMe controller mode (RST/VMD), free space on C:, Wi-Fi chipset if detectable.
- **User data inventory**: sizes of Documents/Desktop/Pictures/Music/Videos/Downloads; browser profiles (Firefox, Chrome, Edge, Brave); Thunderbird; KeePass `.kdbx`; Steam library paths; Discord/other app data worth noting.
- **Mapping**: look each program up in `data/app_map.json` → one of `native` (has a Linux version), `alternative` (list of replacements), `wine_ok` (runs via Wine/Proton, with a link/rating), `blocked` (known not to work, e.g. kernel anti-cheat games, Adobe CC, some Office features), or `unknown`.
- **Output**: a console summary and `report.html` (self-contained, readable, grouped by status, with a top "blockers" section) plus `report.json`.
- Seed `data/app_map.json` with at least 150 common apps (browsers, Office, Adobe, dev tools, games launchers, chat apps, media, utilities, AV, VPNs). Make the file easy to extend by hand. Be honest: mark `unknown` rather than guess.

### Stage 2 — Distro chooser
`linuxify choose` → ask ~10 plain-language questions (experience level, gaming, creative work, old/new hardware, stability vs fresh software, desktop look, privacy, etc.), combine with the scan results (NVIDIA GPU → favour distros with good driver handling; heavy gaming → Bazzite/Nobara-style; old hardware → lightweight), and output a ranked top 3 with reasons. Start with a small, well-chosen set (Linux Mint, Fedora Workstation, Ubuntu, Bazzite, Pop!_OS, Debian, openSUSE Tumbleweed or similar) in `data/distros.json`. The owner mentioned a distro-chooser website (likely distrochooser.de): check whether it has a usable API or permits embedding; if not, use the built-in quiz and say so in `DECISIONS.md`. Do not scrape it.

### Stage 3 — Try before installing
`linuxify try <distro>`:
- Download + SHA-256-verify the ISO into `%LOCALAPPDATA%\Linuxify\isos\`.
- **Option A (default, zero risk): VM trial.** Detect Hyper-V / VirtualBox / QEMU and boot the ISO in one. If none are present, offer to download a portable QEMU.
- **Option B (real hardware, no USB): boot-from-disk trial.** Stage the ISO on a small separate partition or file and add a one-time boot entry (GRUB or Ventoy-on-disk, whichever is most reliable under UEFI + Secure Boot). Must be fully reversible with `linuxify try --undo`. Document the Secure Boot and BitLocker interactions. If this can't be made reliable, ship Option A only and say so.

### Stage 4 — Data migration (staging)
`linuxify migrate --target <path-or-partition>`:
- Copy (never move) user data into a staging layout using Linux-friendly names/paths. Convert what is safely convertible (e.g. Firefox/Chrome profile directories into the form Linux builds expect, KeePass db as-is). Skip/flag anything app-specific that won't carry over.
- Preserve timestamps; write a `manifest.json` with every file's size + SHA-256 so the Linux side (and the later "remove Windows" gate) can verify.
- `--dry-run` first; show totals; resume support for interrupted copies; handle long paths and locked files gracefully.
- Generate a first-boot script (`restore.sh`) that places the data into the new home directory and installs the recommended replacement apps via the distro's package manager / Flatpak.

### Stage 5 — Install (dual boot first)
`linuxify install`: shrink C: (use the Windows storage APIs / `diskpart` scripted, with dry-run), create the Linux partition, stage the pre-seeded installer (Ubuntu autoinstall / Fedora kickstart / Calamares config as appropriate to the chosen distro), set a one-time boot entry, reboot. A separate, heavily gated `--replace-windows` mode comes last and only after the manifest verification passes. **Do not implement Stage 5 against the owner's real disks during development.** Test in a Windows VM with a virtual disk only, and tell the owner clearly what is and isn't tested.

## Model and subagent usage

You are running as the lead (Opus 5.5). You own the architecture, every safety-critical decision, and the final review of everything. Delegate to subagents where it saves time or cost, and use your judgment on when:

- **Haiku subagents** for cheap, mechanical, well-specified work: seeding and extending `data/app_map.json` and `data/distros.json` from a clear schema, formatting/lint passes, bulk file-tree test fixtures, repetitive boilerplate, summarizing docs or logs.
- **Sonnet subagents** for self-contained implementation pieces with a clear interface: e.g. the registry/winget parser, the HTML report renderer, quiz scoring, the manifest hasher, research on a specific distro's autoinstall format or Secure Boot behaviour.
- **Do yourself (Opus):** anything that touches disks, partitions, boot entries, BitLocker/Secure Boot handling, the `--replace-windows` gate, the dry-run/confirmation logic, and the final integration and review of subagent output.

Rules for delegating:
1. Give each subagent a complete brief (files it owns, the exact interface/schema, what "done" looks like, and the safety rules above). Never rely on it having this conversation's context.
2. Two subagents never edit the same files at once.
3. Treat subagent output as unverified: read the diff, build it, and run the tests before accepting it. Anything a Haiku agent produced for the data files gets spot-checked (especially `blocked`/`wine_ok` claims; mark `unknown` rather than keep a guess).
4. Subagents must not run anything destructive or touch real disks or user data; that stays with you, and only after the safety rules are met.

## Working agreements

- Work in order; after each stage: build, run the tests, run it for real on this machine (read-only stages), and write a short result note in `PROGRESS.md` (what works, what was verified, what's untested, known limits).
- Unit-test the pure logic (registry parsing, app mapping, quiz scoring, manifest hashing). Keep OS-touching code thin and behind small functions.
- Prefer boring, explicit code over clever code; no `unsafe` unless there's no alternative, and comment each use.
- Do not commit ISOs, scan outputs, or anything from the user's personal data. `.gitignore` them.
- Do not enter credentials anywhere, do not publish anything, and do not touch the repo's remote/GitHub unless the owner asks.
- Be honest in reports: if a mapping is a guess, an environment wasn't tested, or a stage is only partly working, say so plainly.

## Definition of done per stage

1. Stage 1: `linuxify scan` runs on this machine, `report.html` opens and is accurate on spot-checks (≥10 programs verified by hand).
2. Stage 2: ranked recommendations change sensibly when answers change; tested with at least 5 different answer profiles.
3. Stage 3: VM trial boots a real ISO end to end; `--undo` verified for the boot-entry path if it ships.
4. Stage 4: dry-run and real copy of a test folder tree verify against the manifest; interrupted-copy resume works.
5. Stage 5: full dual-boot install verified in a throwaway VM; the gating logic for `--replace-windows` is tested to refuse when verification fails.

Start with Stage 1.

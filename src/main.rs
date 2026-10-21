use clap::{Parser, Subcommand};
use linuxify::chooser::{self, Answers, DistroList, QUESTIONS};
use linuxify::mapping::{AppMap, Group, Status};
use linuxify::readiness::Severity;
use linuxify::{appdirs, download, install, install_exec, iso, migrate, readiness, report, restore, scan, vm};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "linuxify", version, about = "Helps you move from Windows to Linux, safely.")]
struct Cli {
    /// Folder containing app_map.json / distros.json (default: data/ next to the exe, else the built-in copy)
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read-only scan of programs, hardware and data; writes a readiness report.
    Scan {
        /// Skip `winget list` (registry only).
        #[arg(long)]
        no_winget: bool,
        /// Open report.html in the browser when done.
        #[arg(long)]
        open: bool,
    },
    /// Answer ~10 questions and get your top 3 Linux distros (uses the latest scan if there is one).
    Choose {
        /// Read answers from a JSON file instead of asking (question id → option key).
        #[arg(long)]
        answers: Option<PathBuf>,
        /// Ignore the latest scan report.
        #[arg(long)]
        no_scan: bool,
        /// Open the result page in the browser when done.
        #[arg(long)]
        open: bool,
    },
    /// Try a distro safely in a virtual machine (downloads and verifies the official ISO).
    Try {
        /// Distro id, e.g. linux-mint (default: your top pick from `linuxify choose`).
        distro: Option<String>,
        /// Which VM program to use: virtualbox, hyperv or qemu (default: first one that works).
        #[arg(long)]
        vm: Option<String>,
        /// Also give the VM an empty virtual disk of this many GB, so you can test the installer.
        #[arg(long)]
        disk: Option<u32>,
        /// Use an ISO you already have (it must match the official checksum).
        #[arg(long)]
        iso_file: Option<PathBuf>,
        /// Print exactly what would be downloaded and run, without doing it.
        #[arg(long)]
        dry_run: bool,
        /// Re-check the SHA-256 of an already-downloaded ISO.
        #[arg(long)]
        reverify: bool,
        /// Delete the trial VM for this distro (never touches other VMs).
        #[arg(long)]
        remove: bool,
        /// Show which distros can be downloaded automatically and which VM programs were found.
        #[arg(long)]
        list: bool,
        /// Just download and verify the ISO (no VM needed), e.g. before `linuxify install`.
        #[arg(long)]
        download_only: bool,
    },
    /// Copy (never move) your files into a staging folder for Linux, with checksums and a restore script.
    Migrate {
        /// Folder to stage into (another drive, USB disk or NAS share). A `linuxify-migration` folder is created inside it.
        #[arg(long)]
        target: PathBuf,
        /// Show what would be copied, and the totals, without copying anything.
        #[arg(long)]
        dry_run: bool,
        /// Only re-check an existing staging folder against its manifest.
        #[arg(long)]
        verify: bool,
        /// Skip the re-read check after copying (faster, less safe).
        #[arg(long)]
        no_verify: bool,
        /// Copy just this folder instead of your usual files (can be repeated).
        #[arg(long)]
        from_folder: Vec<PathBuf>,
        /// Testing aid: stop after copying this many files, as if interrupted.
        #[arg(long, hide = true)]
        stop_after: Option<u64>,
    },
    /// Set up a dual-boot install (shrinks C:, keeps Windows). Always try --dry-run first.
    Install {
        /// Distro id (default: your top pick from `linuxify choose`).
        distro: Option<String>,
        /// Space for Linux in GB (default 100, or as much as fits safely).
        #[arg(long)]
        linux_gb: Option<u64>,
        /// Print exactly what would be done, change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Remove the installer partition and boot entry and grow C: back.
        #[arg(long)]
        undo: bool,
        /// Check whether removing Windows can be unlocked (it never deletes anything itself).
        #[arg(long)]
        replace_windows: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Scan { no_winget, open } => run_scan(cli.data_dir, !no_winget, open),
        Command::Choose { answers, no_scan, open } => run_choose(cli.data_dir, answers, no_scan, open),
        Command::Try { distro, vm, disk, iso_file, dry_run, reverify, remove, list, download_only } => {
            run_try(cli.data_dir, TryOpts { distro, vm, disk, iso_file, dry_run, reverify, remove, list, download_only })
        }
        Command::Migrate { target, dry_run, verify, no_verify, from_folder, stop_after } => {
            run_migrate(MigrateOpts { target, dry_run, verify_only: verify, no_verify, from_folder, stop_after })
        }
        Command::Install { distro, linux_gb, dry_run, undo, replace_windows } => {
            run_install(cli.data_dir, InstallOpts { distro, linux_gb, dry_run, undo, replace_windows })
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open_in_browser(path: &Path) {
    // explorer.exe opens the file with the default browser; it does not
    // wait for or depend on our process.
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

fn run_scan(data_dir: Option<PathBuf>, use_winget: bool, open: bool) -> Result<(), String> {
    let (map, source) = AppMap::load(data_dir.as_deref())?;
    println!("Linuxify scan (read-only: nothing on this PC will be changed)\n");
    let r = scan::run(&map, source, &scan::ScanOptions { use_winget }, |msg| println!("  {msg}"));

    let shown: Vec<_> = r.programs.iter().filter(|p| matches!(p.group, Group::Mapped | Group::Unmapped)).collect();
    let count = |s: Status| shown.iter().filter(|p| p.status == s).count();
    println!("\nPrograms: {} found", shown.len());
    for s in [Status::Native, Status::Alternative, Status::WineOk, Status::Blocked, Status::Unknown] {
        println!("  {:<26} {}", s.label(), count(s));
    }
    let blocked: Vec<_> = shown.iter().filter(|p| p.status == Status::Blocked).map(|p| p.program.name.as_str()).collect();
    if !blocked.is_empty() {
        println!("  Won't work on Linux: {}", blocked.join(", "));
    }

    println!("\nReadiness:");
    for f in &r.findings {
        let tag = match f.severity {
            Severity::Blocker => "BLOCKER ",
            Severity::Warning => "HEADS-UP",
            Severity::Info => "info    ",
        };
        println!("  [{tag}] {}", f.title);
    }
    let blockers = r.findings.iter().filter(|f| f.severity == Severity::Blocker).count();
    if blockers == 0 {
        println!("\nNo blockers: this PC is ready for the next steps.");
    } else {
        println!("\n{blockers} blocker(s) must be fixed before installing (details in the report).");
    }

    let dir = report::write(&r, &appdirs::reports_dir()?)?;
    let html = dir.join("report.html");
    println!("\nReport: {}", html.display());
    println!("JSON:   {}", dir.join("report.json").display());
    if open {
        open_in_browser(&html);
    }
    Ok(())
}

/// Asks every question on the terminal. Returns None if the user quits.
fn ask_questions() -> Result<Option<Answers>, String> {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let mut answers = Answers::new();
    println!("A few quick questions (type the number and press Enter, or q to quit).\n");
    for (qi, q) in QUESTIONS.iter().enumerate() {
        println!("{}/{}  {}", qi + 1, QUESTIONS.len(), q.text);
        for (i, o) in q.options.iter().enumerate() {
            println!("     {}) {}", i + 1, o.label);
        }
        loop {
            print!("   > ");
            std::io::stdout().flush().map_err(|e| e.to_string())?;
            let Some(line) = lines.next() else {
                return Ok(None); // input closed
            };
            let line = line.map_err(|e| e.to_string())?;
            // Piped input from PowerShell starts with a UTF-8 byte-order mark.
            let t = line.trim().trim_start_matches('\u{feff}');
            if t.eq_ignore_ascii_case("q") {
                return Ok(None);
            }
            match t.parse::<usize>() {
                Ok(n) if (1..=q.options.len()).contains(&n) => {
                    answers.insert(q.id.to_string(), q.options[n - 1].key.to_string());
                    break;
                }
                _ => println!("   Please type a number from 1 to {}.", q.options.len()),
            }
        }
        println!();
    }
    Ok(Some(answers))
}

fn run_choose(data_dir: Option<PathBuf>, answers_file: Option<PathBuf>, no_scan: bool, open: bool) -> Result<(), String> {
    let (list, _) = DistroList::load(data_dir.as_deref())?;
    let home = appdirs::home()?;

    let scan_path = appdirs::reports_dir()?.join("latest").join("report.json");
    let scan_report: Option<scan::ScanReport> = if no_scan {
        None
    } else {
        match std::fs::read_to_string(&scan_path) {
            Ok(t) => Some(serde_json::from_str(&t).map_err(|e| format!("could not read the latest scan ({}): {e}. Re-run `linuxify scan`.", scan_path.display()))?),
            Err(_) => {
                println!("No scan found, so the result uses your answers only. (Run `linuxify scan` first for a better match.)\n");
                None
            }
        }
    };
    if let Some(r) = &scan_report {
        println!("Using your scan from {}.\n", r.generated_at);
    }
    let hints = scan_report.as_ref().map(chooser::hints_from_scan);

    let answers = match answers_file {
        Some(p) => {
            let t = std::fs::read_to_string(&p).map_err(|e| format!("could not read {}: {e}", p.display()))?;
            let v: serde_json::Value = serde_json::from_str(&t).map_err(|e| format!("{} is not valid JSON: {e}", p.display()))?;
            // Accept a plain answers file or a saved choice.json (answers nested inside).
            let inner = v.get("answers").cloned().unwrap_or(v);
            serde_json::from_value::<Answers>(inner).map_err(|e| format!("{} is not a valid answers file: {e}", p.display()))?
        }
        None => match ask_questions()? {
            Some(a) => a,
            None => {
                println!("Stopped; nothing saved.");
                return Ok(());
            }
        },
    };
    chooser::validate_answers(&answers)?;

    let rec = chooser::rank(&list, &answers, hints.as_ref());

    for n in &rec.scan_notes {
        println!("* {n}");
    }
    if !rec.scan_notes.is_empty() {
        println!();
    }
    println!("Your top 3:\n");
    for (i, r) in rec.ranking.iter().take(3).enumerate() {
        println!("{}. {}  ({:.0}% match · desktop: {})", i + 1, r.name, r.score, r.desktop);
        for x in &r.reasons {
            println!("     + {x}");
        }
        for x in &r.cautions {
            println!("     ! {x}");
        }
        println!("     {}\n", r.homepage);
    }
    if !rec.excluded.is_empty() {
        let names: Vec<_> = rec.excluded.iter().map(|e| format!("{} ({})", e.name, e.why)).collect();
        println!("Left out: {}\n", names.join("; "));
    }

    // Save under %LOCALAPPDATA%\Linuxify only (safety rule 1).
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let saved = serde_json::json!({
        "generated_at": generated_at,
        "scan_used": scan_report.is_some(),
        "answers": answers,
        "top": rec.ranking.iter().take(3).map(|r| r.id.clone()).collect::<Vec<_>>(),
        "recommendation": rec,
    });
    let json_path = home.join("choice.json");
    let html_path = home.join("choice.html");
    std::fs::write(&json_path, serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::write(&html_path, report::render_choice_html(&rec, &generated_at, scan_report.is_some())).map_err(|e| e.to_string())?;
    println!("Saved: {}", html_path.display());
    println!("       {} (your answers are in here; re-use them with --answers)", json_path.display());
    if open {
        open_in_browser(&html_path);
    }
    Ok(())
}

struct TryOpts {
    distro: Option<String>,
    vm: Option<String>,
    disk: Option<u32>,
    iso_file: Option<PathBuf>,
    dry_run: bool,
    reverify: bool,
    remove: bool,
    list: bool,
    download_only: bool,
}

fn latest_scan() -> Option<scan::ScanReport> {
    let p = appdirs::reports_dir().ok()?.join("latest").join("report.json");
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn print_plan(title: &str, steps: &[vm::Step]) {
    println!("{title}");
    for s in steps {
        println!("  {}\n      ({})", s.display(), s.what);
    }
}

fn run_try(data_dir: Option<PathBuf>, o: TryOpts) -> Result<(), String> {
    let (distros, _) = DistroList::load(data_dir.as_deref())?;
    let isos = iso::IsoList::load(data_dir.as_deref())?;
    let home = appdirs::home()?;
    let scan_report = latest_scan();
    let is_admin = scan_report.as_ref().is_some_and(|r| r.hardware.is_admin);
    let backends = vm::detect(is_admin);

    if o.list {
        println!("VM programs found:");
        if backends.is_empty() {
            println!("  none (install VirtualBox: winget install Oracle.VirtualBox)");
        }
        for b in &backends {
            let note = b.unusable.as_ref().map(|u| format!("  [not usable: {u}]")).unwrap_or_default();
            println!("  {:<11} {}{note}", b.backend.label(), b.exe.display());
        }
        println!("\nDistros Linuxify can download and verify for you:");
        for d in &distros.distros {
            let how = if isos.find(&d.id).is_some() { "automatic" } else { "manual: download from the website, then use --iso-file" };
            println!("  {:<22} {how}", d.id);
        }
        return Ok(());
    }

    // Which distro? Default: the top pick from `linuxify choose`.
    let distro_id = match o.distro {
        Some(d) => d,
        None => {
            let text = std::fs::read_to_string(home.join("choice.json"))
                .map_err(|_| "say which distro to try (e.g. `linuxify try linux-mint`), or run `linuxify choose` first".to_string())?;
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            v.pointer("/top/0").and_then(|x| x.as_str()).ok_or("choice.json has no top pick")?.to_string()
        }
    };
    let distro = distros
        .distros
        .iter()
        .find(|d| d.id == distro_id)
        .ok_or_else(|| format!("unknown distro '{distro_id}'. Run `linuxify try --list` to see the ids."))?;

    if o.download_only {
        let src = isos.find(&distro.id).ok_or_else(|| format!("Linuxify can't download {} automatically yet", distro.name))?;
        println!("Checking the official checksum list...");
        let r = download::resolve(src)?;
        println!("  Latest ISO:    {}
  SHA-256:       {}
  Checksum from: {}
", r.file_name, r.sha256, r.checksum_url);
        if o.dry_run {
            println!("Dry run: would download {}", r.url);
            return Ok(());
        }
        let p = download::ensure_iso(&r, &home.join("isos"), o.reverify)?;
        println!("
Ready: {}", p.display());
        return Ok(());
    }

    // Which VM program? Default order: VirtualBox, Hyper-V, QEMU.
    let chosen = match &o.vm {
        Some(name) => {
            let want = vm::Backend::parse(name).ok_or_else(|| format!("unknown VM program '{name}' (use virtualbox, hyperv or qemu)"))?;
            backends.iter().find(|b| b.backend == want).ok_or_else(|| format!("{} was not found on this PC", want.label()))?
        }
        None => [vm::Backend::VirtualBox, vm::Backend::HyperV, vm::Backend::Qemu]
            .iter()
            .find_map(|w| backends.iter().find(|b| b.backend == *w && b.unusable.is_none()))
            .ok_or_else(|| {
                "no usable VM program found. The easiest fix is `winget install Oracle.VirtualBox` (free), then run this again. \
                 Alternatives: QEMU (`winget install SoftwareFreedomConservancy.QEMU`) or Hyper-V from an administrator terminal."
                    .to_string()
            })?,
    };
    if let Some(u) = &chosen.unusable {
        return Err(u.clone());
    }

    if o.remove {
        if chosen.backend != vm::Backend::VirtualBox {
            return Err(format!("--remove is automated for VirtualBox only; delete '{}' in {} yourself", vm::vm_name(&distro.id), chosen.backend.label()));
        }
        let steps = vm::plan_virtualbox_remove(&chosen.exe, &distro.id);
        if o.dry_run {
            print_plan("Dry run: would run", &steps);
            return Ok(());
        }
        if !vm::vbox_vm_exists(&chosen.exe, &distro.id) {
            println!("There is no trial VM for {}.", distro.name);
            return Ok(());
        }
        vm::execute(&steps)?;
        println!("Removed the trial VM for {}. The downloaded ISO was kept in {}.", distro.name, home.join("isos").display());
        return Ok(());
    }

    println!("Trying {} in {} (nothing on your real disks is touched).\n", distro.name, chosen.backend.label());

    // Resolve the official ISO and its checksum.
    let resolved = match isos.find(&distro.id) {
        Some(s) => {
            println!("Checking the official checksum list...");
            let r = download::resolve(s)?;
            println!("  Latest ISO:    {}\n  SHA-256:       {}\n  Checksum from: {}", r.file_name, r.sha256, r.checksum_url);
            Some(r)
        }
        None => None,
    };
    let iso_dir = home.join("isos");
    let iso_path: PathBuf = match (&o.iso_file, &resolved) {
        (Some(local), Some(r)) if o.dry_run => {
            println!("\nDry run: would check {} against the official SHA-256 above ({}).", local.display(), r.sha256);
            local.clone()
        }
        (Some(local), Some(r)) => download::verify_local(local, r)?,
        (Some(local), None) => {
            println!(
                "\nWarning: Linuxify has no official checksum source for {} yet, so it cannot verify {}. \
                 Compare it yourself with the checksum on {} before trusting it.",
                distro.name,
                local.display(),
                distro.homepage
            );
            local.clone()
        }
        (None, Some(r)) if o.dry_run => {
            println!("\nDry run: would download\n  {}\n  to {}\n  and refuse to use it unless its SHA-256 matches.", r.url, iso_dir.join(&r.file_name).display());
            iso_dir.join(&r.file_name)
        }
        (None, Some(r)) => {
            println!();
            download::ensure_iso(r, &iso_dir, o.reverify)?
        }
        (None, None) => {
            return Err(format!(
                "Linuxify can't download {} automatically yet. Download the ISO from {} and run:\n  linuxify try {} --iso-file <path-to-iso>",
                distro.name, distro.homepage, distro.id
            ))
        }
    };

    // Plan the VM.
    let (memory_mb, cpus) = vm::size_for_host(
        scan_report.as_ref().and_then(|r| r.hardware.ram_bytes).map(|b| b as f64 / (1u64 << 30) as f64),
        scan_report.as_ref().and_then(|r| r.hardware.cpus.first()).and_then(|c| c.threads),
    );
    let spec = vm::TrialSpec { distro_id: distro.id.clone(), iso: iso_path, memory_mb, cpus, disk_gb: o.disk, vm_dir: home.join("vms") };
    let steps = match chosen.backend {
        vm::Backend::VirtualBox => vm::plan_virtualbox(&chosen.exe, &spec, !o.dry_run && vm::vbox_vm_exists(&chosen.exe, &distro.id)),
        vm::Backend::Qemu => vm::plan_qemu(&chosen.exe, &spec),
        vm::Backend::HyperV => vm::plan_hyperv(&chosen.exe, &spec, !o.dry_run && vm::hyperv_vm_exists(&distro.id)),
    };
    if o.dry_run {
        let disk = o.disk.map(|g| format!(", {g} GB virtual disk")).unwrap_or_default();
        print_plan(&format!("\nDry run: would run these {} commands ({memory_mb} MB RAM, {cpus} CPUs{disk}):", chosen.backend.label()), &steps);
        return Ok(());
    }
    std::fs::create_dir_all(&spec.vm_dir).map_err(|e| e.to_string())?;
    println!("\nStarting the VM ({memory_mb} MB RAM, {cpus} CPUs):");
    vm::execute(&steps)?;
    println!(
        "\nThe {} live session should now be open in a window. Nothing is installed anywhere; \
         just power the VM off when you're done.\nTo delete the trial VM later: linuxify try {} --remove",
        distro.name, distro.id
    );
    if distro.secure_boot != chooser::SecureBootSupport::Supported {
        println!("Note: the VM runs with Secure Boot off, so it doesn't show how {} behaves with Secure Boot on.", distro.name);
    }
    Ok(())
}

struct MigrateOpts {
    target: PathBuf,
    dry_run: bool,
    verify_only: bool,
    no_verify: bool,
    from_folder: Vec<PathBuf>,
    stop_after: Option<u64>,
}

/// File system of the target's drive (e.g. "NTFS", "FAT32"); None for network paths.
fn drive_filesystem(p: &Path) -> Option<String> {
    let s = p.to_string_lossy();
    let letter = s.chars().next().filter(|c| c.is_ascii_alphabetic() && s[1..].starts_with(':'))?;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &format!("(Get-Volume -DriveLetter {letter}).FileSystem")])
        .output()
        .ok()?;
    let fs = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!fs.is_empty()).then_some(fs)
}

fn print_verify(r: &migrate::VerifyReport) -> bool {
    println!("  {} files match the manifest.", r.ok);
    for (label, list) in [("MISSING", &r.missing), ("DIFFERENT FROM MANIFEST", &r.mismatched), ("not in manifest (extra)", &r.extra)] {
        if !list.is_empty() {
            println!("  {} {label}:", list.len());
            for p in list.iter().take(20) {
                println!("    {p}");
            }
            if list.len() > 20 {
                println!("    ... and {} more", list.len() - 20);
            }
        }
    }
    r.missing.is_empty() && r.mismatched.is_empty()
}

fn run_migrate(o: MigrateOpts) -> Result<(), String> {
    if !o.target.is_dir() {
        return Err(format!("{} is not an existing folder. Pick a folder on another drive, a USB disk or a NAS share.", o.target.display()));
    }
    let staging = o.target.join(migrate::STAGING_DIR);

    if o.verify_only {
        println!("Checking {} against its manifest...", staging.display());
        let r = migrate::verify(&staging, |_, _| {})?;
        return if print_verify(&r) { Ok(()) } else { Err("verification FAILED".into()) };
    }

    let sources: Vec<migrate::SourceItem> = if o.from_folder.is_empty() {
        println!("Finding your files...");
        migrate::default_sources(&scan::userdata::scan())
    } else {
        o.from_folder
            .iter()
            .map(|f| {
                let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Folder".into());
                migrate::SourceItem { label: name.clone(), from: f.clone(), to: format!("home/{name}"), exclude: vec![], only: vec![] }
            })
            .collect()
    };
    migrate::check_target(&staging, &sources)?;

    println!("Planning the copy (reading file sizes only)...");
    let plan = migrate::plan(&sources);
    println!("\n{}Copy plan -> {}", if o.dry_run { "DRY RUN: " } else { "" }, staging.display());
    for (label, (files, bytes)) in &plan.per_source {
        println!("{}", migrate::summary_line(label, *files, *bytes));
    }
    let total = plan.total_bytes();
    println!("{}", migrate::summary_line("TOTAL", plan.files.len() as u64, total));

    if !plan.skipped.is_empty() {
        let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
        for s in &plan.skipped {
            *reasons.entry(s.reason.as_str()).or_default() += 1;
        }
        println!("\nWill NOT copy {} items:", plan.skipped.len());
        for (r, n) in reasons {
            println!("  {n:>6}  {r}");
        }
    }

    let free = fs2::available_space(&o.target).ok();
    if let Some(f) = free {
        println!("\nFree space on the target: {}", download::human(f));
    }
    let fs = drive_filesystem(&o.target);
    if fs.as_deref() == Some("FAT32") && plan.files.iter().any(|f| f.size >= 4 * 1024 * 1024 * 1024) {
        return Err("the target drive is FAT32, which can't hold files of 4 GB or more; use an exFAT or NTFS drive".into());
    }
    if o.target.to_string_lossy().to_ascii_uppercase().starts_with("C:") {
        println!("Note: the target is on C:, the Windows drive. Linux can read it in a dual boot only if BitLocker and Fast Startup are off.");
    }
    if o.dry_run {
        println!("\nDry run: nothing was copied. Run the same command without --dry-run to copy.");
        return Ok(());
    }
    let already = std::fs::read_to_string(staging.join(migrate::PROGRESS_FILE)).map(|t| migrate::parse_progress(&t)).unwrap_or_default();
    let reusable: u64 = plan.files.iter().filter(|f| already.get(&f.rel).is_some_and(|c| c.size == f.size && c.mtime == f.mtime)).map(|f| f.size).sum();
    if let Some(f) = free {
        if total - reusable + 512 * 1024 * 1024 > f {
            return Err(format!("not enough space on the target: need {} more, {} free", download::human(total - reusable), download::human(f)));
        }
    }

    println!("\nCopying (your originals are only read, never changed)...");
    let mut last = std::time::Instant::now();
    let out = migrate::copy_all(&plan, &staging, o.stop_after, |done, total, _| {
        if last.elapsed() > std::time::Duration::from_secs(2) {
            print!("\r  {:>3}%  {} / {}   ", done * 100 / total.max(1), download::human(done), download::human(total));
            let _ = std::io::stdout().flush();
            last = std::time::Instant::now();
        }
    })?;
    println!("\r  Copied {} new files, reused {} from an earlier run.          ", out.newly_copied, out.reused);
    let mut skipped = plan.skipped.clone();
    skipped.extend(out.failed.iter().cloned());

    let manifest = migrate::write_manifest(&staging, &out.copied, &skipped)?;
    // Linux side: restore.sh must keep Unix line endings.
    std::fs::write(staging.join("restore.sh"), restore::RESTORE_SH.replace("\r\n", "\n")).map_err(|e| e.to_string())?;
    std::fs::write(staging.join("remove-windows.sh"), restore::REMOVE_WINDOWS_SH.replace("\r\n", "\n")).map_err(|e| e.to_string())?;
    let programs = latest_scan().map(|r| r.programs).unwrap_or_default();
    let lines = restore::app_lines(&programs);
    std::fs::write(staging.join("apps.txt"), restore::apps_txt(&lines)).map_err(|e| e.to_string())?;
    let distro = std::fs::read_to_string(appdirs::home()?.join("choice.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.pointer("/top/0").and_then(|x| x.as_str()).map(String::from));
    std::fs::write(staging.join("README.txt"), restore::readme_txt(distro.as_deref(), manifest.total_files, &download::human(manifest.total_bytes))).map_err(|e| e.to_string())?;

    // Remember where the staged copy is (Stage 5's "remove Windows" gate needs it).
    let home = appdirs::home()?;
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let record = serde_json::json!({
        "staging": staging.display().to_string(),
        "created_at": manifest.created_at,
        "total_files": manifest.total_files,
        "total_bytes": manifest.total_bytes,
    });
    std::fs::write(home.join("migration.json"), serde_json::to_string_pretty(&record).unwrap()).map_err(|e| e.to_string())?;

    if !out.failed.is_empty() {
        println!("\n{} files could not be copied (see flagged.txt), e.g.:", out.failed.len());
        for f in out.failed.iter().take(5) {
            println!("  {}  ({})", f.from, f.reason);
        }
        println!("Close the programs using them and run the same command again: finished files are not copied twice.");
    }

    if !o.no_verify {
        println!("\nVerifying the copy against the manifest (re-reading every file)...");
        let r = migrate::verify(&staging, |_, _| {})?;
        if !print_verify(&r) {
            return Err("verification FAILED: some copied files don't match. Run the command again to re-copy them.".into());
        }
    }
    println!(
        "\nDone: {} files ({}) staged in {}\n{} apps listed in apps.txt. On Linux, open that folder and run:  bash restore.sh --dry-run",
        manifest.total_files,
        download::human(manifest.total_bytes),
        staging.display(),
        lines.len()
    );
    Ok(())
}

struct InstallOpts {
    distro: Option<String>,
    linux_gb: Option<u64>,
    dry_run: bool,
    undo: bool,
    replace_windows: bool,
}

fn read_typed_line() -> String {
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    s.trim().trim_start_matches('\u{feff}').to_string()
}

/// Verifies the staged copy from `linuxify migrate`; returns (report, staging path).
fn verify_recorded_migration() -> Option<(migrate::VerifyReport, PathBuf, u64)> {
    let rec: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(appdirs::home().ok()?.join("migration.json")).ok()?).ok()?;
    let staging = PathBuf::from(rec.get("staging")?.as_str()?);
    let total = rec.get("total_files").and_then(|v| v.as_u64()).unwrap_or(0);
    println!("Verifying your staged copy in {} (re-reading every file)...", staging.display());
    let r = migrate::verify(&staging, |_, _| {}).ok()?;
    Some((r, staging, total))
}

fn run_install(data_dir: Option<PathBuf>, o: InstallOpts) -> Result<(), String> {
    if o.replace_windows {
        return run_replace_windows();
    }
    if o.undo {
        let state = install_exec::load_state().ok_or("there is no Linuxify install setup to undo")?;
        let p = &state.plan;
        println!("Undo will: delete the {} boot entry, delete the {} installer partition (disk {}, at {} GB), and grow C: back into the free space.", install::BCD_DESCRIPTION, install::INSTALLER_LABEL, p.disk, p.installer_offset / install::GB);
        println!("It does NOT remove a Linux system if you already installed one into the free space.");
        if o.dry_run {
            for s in install_exec::undo(&state, true)? {
                println!("  would run: {s}");
            }
            return Ok(());
        }
        let lock = appdirs::home()?.join("no-install.lock");
        if lock.exists() {
            return Err(format!("this PC is locked against disk changes ({} exists); nothing was changed", lock.display()));
        }
        let phrase = format!("UNDO LINUXIFY ON DISK {}", p.disk);
        println!("\nType exactly  {phrase}  to continue:");
        if read_typed_line() != phrase {
            return Err("confirmation didn't match; nothing was changed".into());
        }
        for s in install_exec::undo(&state, false)? {
            println!("  done: {s}");
        }
        println!("Undo finished.");
        return Ok(());
    }

    // ---- Which distro and ISO ----
    let (distros, _) = DistroList::load(data_dir.as_deref())?;
    let home = appdirs::home()?;
    let distro_id = match o.distro {
        Some(d) => d,
        None => {
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(home.join("choice.json")).map_err(|_| "say which distro to install, or run `linuxify choose` first".to_string())?).map_err(|e| e.to_string())?;
            v.pointer("/top/0").and_then(|x| x.as_str()).ok_or("choice.json has no top pick")?.to_string()
        }
    };
    let distro = distros.distros.iter().find(|d| d.id == distro_id).ok_or(format!("unknown distro '{distro_id}'"))?;
    let isos = iso::IsoList::load(data_dir.as_deref())?;
    let src = isos.find(&distro.id).ok_or(format!("Linuxify can't fetch a verified ISO for {} yet, so it won't install it. Use a USB stick.", distro.name))?;
    println!("Linuxify install: {} as a DUAL BOOT (Windows stays).\n", distro.name);
    println!("Checking the official ISO...");
    let resolved = download::resolve(src)?;
    let iso_path = home.join("isos").join(&resolved.file_name);
    let iso_verified = download::already_verified(&iso_path, &resolved.sha256);
    let iso_bytes = std::fs::metadata(&iso_path).map(|m| m.len()).unwrap_or(resolved_size_guess(&resolved));

    // ---- Fresh scan + gates ----
    println!("Re-scanning disks and boot settings...");
    let (hw, hw_err) = scan::hardware::scan();
    let findings = readiness::assess(&hw, hw_err.as_deref(), &[], &scan::userdata::UserData::default());
    let backup = verify_recorded_migration();
    let windows_disk = hw.disks.iter().find(|d| d.partitions.iter().any(|p| p.drive_letter == Some('C'))).map(|d| d.number);
    let backup_ok = backup.as_ref().map(|(r, _, _)| r.missing.is_empty() && r.mismatched.is_empty() && r.ok > 0);
    let backup_on_windows_disk = backup.as_ref().is_some_and(|(_, staging, _)| {
        windows_disk.is_some() && staging.to_string_lossy().chars().next().and_then(|l| install_exec::disk_of_letter(&hw, l)) == windows_disk
    });
    let pre = install::Preflight {
        is_admin: hw.is_admin,
        blockers: install::blockers_from(&findings),
        iso_verified,
        backup_verified: backup_ok,
        backup_on_windows_disk,
        already_installed_state: install_exec::load_state().is_some(),
    };
    let mut problems = install::preflight_problems(&pre);
    // A machine owner can lock Linuxify out of changing disks entirely.
    let lock = home.join("no-install.lock");
    if lock.exists() {
        problems.insert(0, format!("this PC is locked against installs ({} exists); delete that file only if you really want Linuxify to change this PC's disks", lock.display()));
    }

    // ---- Plan ----
    let letter = install_exec::free_letter(&hw).ok_or("no free drive letter for the installer partition")?;
    let req = install::InstallRequest { distro_id: distro.id.clone(), iso: iso_path.display().to_string(), iso_bytes, largest_iso_file: 0, linux_gb: o.linux_gb, free_letter: letter };
    let mut plan = install::plan(&hw, &req);
    let mut estimate = false;
    if o.dry_run && hw.c_shrinkable.is_none() {
        // Preview only: without admin, assume C: can shrink by its free space.
        let mut guess = hw.clone();
        guess.c_shrinkable = hw.disks.iter().flat_map(|d| d.partitions.iter()).find(|p| p.drive_letter == Some('C')).and_then(|p| p.free);
        plan = install::plan(&guess, &req);
        estimate = true;
    }

    println!();
    match &plan {
        Ok(p) => {
            println!("PLAN (disk {}){}:", p.disk, if estimate { " - ESTIMATE: run as administrator for the exact shrink size" } else { "" });
            println!("  1. Shrink C: from {} GB to {} GB (frees {} GB). Windows and your files stay.", p.c_size_before / install::GB, p.c_size_after / install::GB, p.shrink / install::GB);
            println!("  2. Create a {} GB FAT32 partition '{}' right after C: and copy the verified {} installer onto it.", p.installer_size / install::GB, install::INSTALLER_LABEL, distro.name);
            println!("  3. Leave {} GB unallocated for Linux.", p.linux_size / install::GB);
            println!("  4. Add a ONE-TIME boot entry so the next restart starts the installer (later restarts go to Windows as usual).");
            println!("\n  diskpart script:\n    {}", p.diskpart_create().trim_end().replace("\r\n", "\n    "));
            println!("  then: robocopy <ISO> {}:\\ ; bcdedit /copy {{bootmgr}} /d \"{}\" ; bcdedit /set <id> device partition={}: ; bcdedit /set <id> path \\EFI\\BOOT\\BOOTX64.EFI ; bcdedit /set {{fwbootmgr}} bootsequence <id> ; diskpart remove letter={}", p.letter, install::BCD_DESCRIPTION, p.letter, p.letter);
            println!("\n  In the installer, choose 'Install alongside Windows' or 'use the free space'. Do NOT choose 'Erase disk'.");
        }
        Err(why) => {
            println!("Linuxify can't make a plan:");
            for w in why {
                println!("  - {w}");
            }
        }
    }
    if !problems.is_empty() {
        println!("\n{} it would refuse to run because:", if o.dry_run { "Dry run:" } else { "Stopped:" });
        for pr in &problems {
            println!("  - {pr}");
        }
    }
    if o.dry_run {
        println!("\nDry run: nothing was changed.");
        return Ok(());
    }
    let plan = plan.map_err(|w| w.join("; "))?;
    if !problems.is_empty() {
        return Err("not safe to continue; nothing was changed".into());
    }

    // ---- Typed confirmation ----
    let phrase = plan.confirm_phrase();
    println!("\nThis changes your disk. Your files were verified on {} and Windows will stay installed.", backup.as_ref().map(|b| b.1.display().to_string()).unwrap_or_default());
    println!("Type exactly  {phrase}  to continue (anything else cancels):");
    if read_typed_line() != phrase {
        return Err("confirmation didn't match; nothing was changed".into());
    }
    let state = install_exec::execute(&plan)?;
    println!("\nReady (step: {:?}). The NEXT restart will start the {} installer; later restarts go to Windows as usual.", state.phase, distro.name);
    println!("Restart when you're ready. In the installer pick 'Install alongside Windows' / free space.");
    println!("Changed your mind? `linuxify install --undo` puts it back.");
    Ok(())
}

fn resolved_size_guess(_r: &download::ResolvedIso) -> u64 {
    // Only used for a dry run before the ISO is downloaded: assume a 6 GB ISO.
    6 * install::GB
}

/// Windows-side half of "remove Windows": refuses unless the staged copy
/// verifies completely (count + hashes) and doesn't live on the Windows disk.
/// It never deletes anything itself.
fn run_replace_windows() -> Result<(), String> {
    println!("Checking whether it's safe to remove Windows later...\n");
    let (hw, _) = scan::hardware::scan();
    let windows_disk = hw.disks.iter().find(|d| d.partitions.iter().any(|p| p.drive_letter == Some('C'))).map(|d| d.number);
    let gate = match verify_recorded_migration() {
        None => install::ReplaceGate::default(),
        Some((r, staging, total)) => install::ReplaceGate {
            migration_recorded: true,
            staging_files_ok: r.ok,
            staging_missing: r.missing.len() as u64,
            staging_mismatched: r.mismatched.len() as u64,
            manifest_files: total,
            staging_on_windows_disk: windows_disk.is_some() && staging.to_string_lossy().chars().next().and_then(|l| install_exec::disk_of_letter(&hw, l)) == windows_disk,
        },
    };
    let problems = install::replace_gate_problems(&gate);
    if !problems.is_empty() {
        println!("REFUSED: removing Windows stays locked because:");
        for p in &problems {
            println!("  - {p}");
        }
        return Err("replace-windows gate failed".into());
    }
    println!("Your staged copy verified completely ({} files).", gate.staging_files_ok);
    println!("Removing Windows is done from Linux, after you've restored your files there:");
    println!("  1. Boot into Linux, open the linuxify-migration folder, run:  bash restore.sh");
    println!("  2. Then:  bash remove-windows.sh   (it re-checks every restored file before it will touch Windows,");
    println!("     shows exactly which partitions it would delete, and needs a typed confirmation).");
    Ok(())
}

use clap::{Parser, Subcommand};
use linuxify::chooser::{self, Answers, DistroList, QUESTIONS};
use linuxify::mapping::{AppMap, Group, Status};
use linuxify::readiness::Severity;
use linuxify::flow::{self, latest_scan};
use linuxify::{appdirs, download, install, install_exec, iso, migrate, report, scan, vm};
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
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the Linuxify window (the default when no command is given).
    Gui {
        /// Page to open on: welcome, scan, choose, try, migrate or install.
        #[arg(long)]
        page: Option<String>,
    },
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
    let Some(command) = cli.command else {
        return run_gui(cli.data_dir, None);
    };
    let result = match command {
        Command::Gui { page } => return run_gui(cli.data_dir, page),
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

fn run_gui(data_dir: Option<PathBuf>, page: Option<String>) -> ExitCode {
    if linuxify::gui::restart_without_console() {
        return ExitCode::SUCCESS;
    }
    match linuxify::gui::run(data_dir, page.as_deref()) {
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

    // Saved under %LOCALAPPDATA%\Linuxify only (safety rule 1).
    let (json_path, html_path) = flow::save_choice(&answers, &rec, scan_report.is_some())?;
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
        None => flow::top_pick().ok_or("say which distro to try (e.g. `linuxify try linux-mint`), or run `linuxify choose` first")?,
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

    let chosen = flow::pick_backend(&backends, o.vm.as_deref())?;

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
    let spec = flow::trial_spec(&distro.id, iso_path, o.disk)?;
    let (memory_mb, cpus) = (spec.memory_mb, spec.cpus);
    let steps = flow::trial_steps(chosen, &spec, o.dry_run);
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
    flow::check_target_filesystem(&o.target, &plan)?;
    if o.target.to_string_lossy().to_ascii_uppercase().starts_with("C:") {
        println!("Note: the target is on C:, the Windows drive. Linux can read it in a dual boot only if BitLocker and Fast Startup are off.");
    }
    if o.dry_run {
        println!("\nDry run: nothing was copied. Run the same command without --dry-run to copy.");
        return Ok(());
    }
    let reusable = flow::reusable_bytes(&plan, &staging);
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
    let (manifest, app_count) = flow::finish_staging(&staging, &out, &plan.skipped)?;

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
        app_count
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

fn run_install(data_dir: Option<PathBuf>, o: InstallOpts) -> Result<(), String> {
    if o.replace_windows {
        return run_replace_windows();
    }
    if o.undo {
        let state = install_exec::load_state().ok_or("there is no Linuxify install setup to undo")?;
        println!("{}", flow::undo_summary(&state));
        println!("It does NOT remove a Linux system if you already installed one into the free space.");
        if o.dry_run {
            for s in install_exec::undo(&state, true)? {
                println!("  would run: {s}");
            }
            return Ok(());
        }
        if let Some(lock) = flow::install_lock() {
            return Err(format!("this PC is locked against disk changes ({} exists); nothing was changed", lock.display()));
        }
        println!("\nType exactly  {}  to continue:", flow::undo_phrase(&state));
        for s in flow::undo_confirmed(&state, &read_typed_line())? {
            println!("  done: {s}");
        }
        println!("Undo finished.");
        return Ok(());
    }

    // ---- Which distro ----
    let distro_id = match o.distro {
        Some(d) => d,
        None => flow::top_pick().ok_or("say which distro to install, or run `linuxify choose` first")?,
    };
    let (distros, _) = DistroList::load(data_dir.as_deref())?;
    let name = flow::find_distro(&distros, &distro_id)?.name.clone();
    println!("Linuxify install: {name} as a DUAL BOOT (Windows stays).\n");

    // ---- Fresh scan, gates and plan (changes nothing) ----
    let prep = flow::prepare_install(data_dir.as_deref(), &distro_id, o.linux_gb, o.dry_run, &mut |m| println!("{m}"), &mut |_, _| {})?;
    let distro = &prep.distro;

    println!();
    match &prep.plan {
        Ok(p) => {
            println!("PLAN (disk {}){}:", p.disk, if prep.estimate { " - ESTIMATE: run as administrator for the exact shrink size" } else { "" });
            for (i, step) in flow::plan_steps(p, &distro.name).iter().enumerate() {
                println!("  {}. {step}", i + 1);
            }
            println!("\n  {}", flow::plan_commands(p).replace('\n', "\n  "));
            println!("\n  {}", install::installer_hint(distro.installer, p.disk_size));
        }
        Err(why) => {
            println!("Linuxify can't make a plan:");
            for w in why {
                println!("  - {w}");
            }
        }
    }
    if !prep.problems.is_empty() {
        println!("\n{} it would refuse to run because:", if o.dry_run { "Dry run:" } else { "Stopped:" });
        for pr in &prep.problems {
            println!("  - {pr}");
        }
    }
    if o.dry_run {
        println!("\nDry run: nothing was changed.");
        return Ok(());
    }
    let plan = prep.plan.as_ref().map_err(|w| w.join("; "))?;
    if !prep.problems.is_empty() {
        return Err("not safe to continue; nothing was changed".into());
    }

    // ---- Typed confirmation ----
    println!(
        "\nThis changes your disk. Your files were verified on {} and Windows will stay installed.",
        prep.backup_staging.as_ref().map(|b| b.display().to_string()).unwrap_or_default()
    );
    println!("Type exactly  {}  to continue (anything else cancels):", plan.confirm_phrase());
    let state = flow::install_confirmed(&prep, &read_typed_line())?;
    println!("\nReady (step: {:?}). The NEXT restart will start the {} installer; later restarts go to Windows as usual.", state.phase, distro.name);
    println!("Restart when you're ready. {}", install::installer_hint(distro.installer, plan.disk_size));
    println!("Changed your mind? `linuxify install --undo` puts it back.");
    println!("After Linux is installed, the same command (run from Windows) removes the {} GB installer partition{} and gives the space back to C:. Linux stays.", plan.installer_size / install::GB, if plan.data_size > 0 { "s" } else { "" });
    Ok(())
}

/// Windows-side half of "remove Windows": refuses unless the staged copy
/// verifies completely (count + hashes) and doesn't live on the Windows disk.
/// It never deletes anything itself.
fn run_replace_windows() -> Result<(), String> {
    println!("Checking whether it's safe to remove Windows later...\n");
    if let Some((staging, _)) = flow::recorded_migration() {
        println!("Verifying your staged copy in {} (re-reading every file)...", staging.display());
    }
    let (gate, problems) = flow::replace_windows_gate(|_, _| {});
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

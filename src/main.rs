use clap::{Parser, Subcommand};
use linuxify::chooser::{self, Answers, DistroList, QUESTIONS};
use linuxify::mapping::{AppMap, Group, Status};
use linuxify::readiness::Severity;
use linuxify::{appdirs, download, iso, report, scan, vm};
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
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Scan { no_winget, open } => run_scan(cli.data_dir, !no_winget, open),
        Command::Choose { answers, no_scan, open } => run_choose(cli.data_dir, answers, no_scan, open),
        Command::Try { distro, vm, disk, iso_file, dry_run, reverify, remove, list } => {
            run_try(cli.data_dir, TryOpts { distro, vm, disk, iso_file, dry_run, reverify, remove, list })
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

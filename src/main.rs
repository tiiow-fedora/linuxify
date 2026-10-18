use clap::{Parser, Subcommand};
use linuxify::mapping::{AppMap, Group, Status};
use linuxify::readiness::Severity;
use linuxify::{appdirs, report, scan};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "linuxify", version, about = "Helps you move from Windows to Linux, safely.")]
struct Cli {
    /// Folder containing app_map.json (default: data/ next to the exe, else the built-in copy)
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan { no_winget, open } => match run_scan(cli.data_dir, !no_winget, open) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
    }
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
        // explorer.exe opens the file with the default browser; it does not
        // wait for or depend on our process.
        let _ = std::process::Command::new("explorer.exe").arg(&html).spawn();
    }
    Ok(())
}

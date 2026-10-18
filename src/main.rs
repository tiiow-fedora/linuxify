use clap::{Parser, Subcommand};
use linuxify::chooser::{self, Answers, DistroList, QUESTIONS};
use linuxify::mapping::{AppMap, Group, Status};
use linuxify::readiness::Severity;
use linuxify::{appdirs, report, scan};
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
        /// Read answers from a JSON file instead of asking (question id â†’ option key).
        #[arg(long)]
        answers: Option<PathBuf>,
        /// Ignore the latest scan report.
        #[arg(long)]
        no_scan: bool,
        /// Open the result page in the browser when done.
        #[arg(long)]
        open: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Scan { no_winget, open } => run_scan(cli.data_dir, !no_winget, open),
        Command::Choose { answers, no_scan, open } => run_choose(cli.data_dir, answers, no_scan, open),
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

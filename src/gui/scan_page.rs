//! Stage 1: the read-only scan and its readiness report.

use super::job::Task;
use super::style::*;
use super::{next_button, open_path, Page, Shared};
use crate::mapping::{AppMap, Group, Status};
use crate::readiness::Severity;
use crate::scan::hardware::{BitLocker, Firmware, SecureBoot};
use crate::{appdirs, download, report, scan};
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct ScanPage {
    task: Task<std::path::PathBuf>,
    no_winget: bool,
    filter: Option<Status>,
    search: String,
}

const STATUSES: [(Status, &str); 5] = [
    (Status::Native, "Native on Linux"),
    (Status::Alternative, "Linux alternative"),
    (Status::WineOk, "Wine / Proton"),
    (Status::Blocked, "Won't work"),
    (Status::Unknown, "Unknown"),
];

fn status_color(s: Status) -> egui::Color32 {
    match s {
        Status::Native => ACCENT,
        Status::Alternative => BLUE,
        Status::WineOk => egui::Color32::from_rgb(180, 140, 255),
        Status::Blocked => RED,
        Status::Unknown => FAINT,
    }
}

impl ScanPage {
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, sh: &mut Shared) -> Option<Page> {
        if self.task.tick() {
            sh.refresh();
        }
        header(ui, "STEP 1", "Scan this PC", "Finds your programs, files and hardware and checks what works on Linux. Nothing on this PC is changed.", Some(("Read-only", ACCENT)));

        if let Task::Running(job) = &self.task {
            progress(ui, &job.progress(), job.started.elapsed());
            return None;
        }
        if let Some(e) = self.task.err() {
            error_box(ui, "The scan didn't finish", e);
            ui.add_space(8.0);
        }

        let Some(r) = sh.scan.clone() else {
            card(ui, |ui| {
                ui.label(RichText::new("Ready when you are").font(bold(17.0)).color(TEXT));
                muted(ui, "Takes 10–30 seconds. Linuxify reads the installed-programs list, hardware and boot settings, and the size of your folders. It doesn't open or upload your files.");
                ui.add_space(8.0);
                ui.checkbox(&mut self.no_winget, "Skip winget (faster, but misses Microsoft Store apps)");
                ui.add_space(8.0);
                if primary(ui, "Scan this PC", true).clicked() {
                    self.start(ctx, sh);
                }
            });
            return None;
        };

        // ---- Summary ----
        ui.horizontal(|ui| {
            muted(ui, &format!("Scanned {}{}.", r.generated_at, if r.winget_used { " (registry + winget)" } else { " (registry only)" }));
        });
        ui.add_space(6.0);
        let shown: Vec<_> = r.programs.iter().filter(|p| matches!(p.group, Group::Mapped | Group::Unmapped)).collect();
        let count = |s: Status| shown.iter().filter(|p| p.status == s).count();
        ui.columns(6, |cols| {
            stat(&mut cols[0], &shown.len().to_string(), "programs found", TEXT);
            for (col, (s, label)) in cols[1..].iter_mut().zip(STATUSES) {
                stat(col, &count(s).to_string(), label, status_color(s));
            }
        });

        // ---- Readiness ----
        let blockers = r.findings.iter().filter(|f| f.severity == Severity::Blocker).count();
        section(ui, "Is this PC ready for Linux?");
        if blockers == 0 {
            tinted(ui, ACCENT, |ui| {
                ui.label(RichText::new("✔  No blockers: this PC is ready for the next steps.").font(bold(15.5)).color(ACCENT));
            });
        } else {
            tinted(ui, RED, |ui| {
                ui.label(RichText::new(format!("{blockers} blocker(s) must be fixed before installing.")).font(bold(15.5)).color(RED));
                muted(ui, "You can still scan, choose, try Linux in a VM and copy your files.");
            });
        }
        ui.add_space(4.0);
        let mut findings = r.findings.clone();
        findings.sort_by_key(|f| match f.severity {
            Severity::Blocker => 0,
            Severity::Warning => 1,
            Severity::Info => 2,
        });
        for f in &findings {
            let (color, tag) = match f.severity {
                Severity::Blocker => (RED, "BLOCKER"),
                Severity::Warning => (AMBER, "HEADS-UP"),
                Severity::Info => (BLUE, "INFO"),
            };
            ui.add_space(6.0);
            tinted(ui, color, |ui| {
                ui.horizontal(|ui| {
                    pill(ui, tag, color);
                    ui.label(RichText::new(&f.title).font(bold(15.0)).color(TEXT));
                });
                if !f.detail.is_empty() {
                    muted(ui, &f.detail);
                }
                if let Some(fix) = &f.how_to_fix {
                    bullet(ui, "→", color, fix);
                }
            });
        }

        // ---- Hardware ----
        section(ui, "This PC");
        let hw = &r.hardware;
        card(ui, |ui| {
            egui::Grid::new("hw").num_columns(2).spacing([28.0, 8.0]).show(ui, |ui| {
                let row = |ui: &mut egui::Ui, k: &str, v: String| {
                    ui.label(RichText::new(k).color(MUTED));
                    ui.add(egui::Label::new(RichText::new(v).color(TEXT)).wrap());
                    ui.end_row();
                };
                if let Some(os) = &hw.os {
                    row(ui, "Windows", format!("{} {}", os.caption.clone().unwrap_or_default(), os.build.as_deref().map(|b| format!("(build {b})")).unwrap_or_default()));
                }
                if let Some(c) = hw.cpus.first() {
                    row(ui, "Processor", format!("{}{}", c.name.clone().unwrap_or_else(|| "unknown".into()), c.threads.map(|t| format!(" · {t} threads")).unwrap_or_default()));
                }
                row(ui, "Memory", hw.ram_bytes.map(|b| format!("{:.0} GB", b as f64 / (1u64 << 30) as f64)).unwrap_or_else(|| "unknown".into()));
                let gpus: Vec<_> = hw.gpus.iter().filter(|g| !g.is_virtual).map(|g| g.name.clone()).collect();
                row(ui, "Graphics", if gpus.is_empty() { "none found".into() } else { gpus.join(", ") });
                row(ui, "Firmware", match hw.firmware { Firmware::Uefi => "UEFI".into(), Firmware::Bios => "Legacy BIOS".into(), Firmware::Unknown => "unknown".into() });
                row(ui, "Secure Boot", match hw.secure_boot { SecureBoot::On => "On".into(), SecureBoot::Off => "Off".into(), SecureBoot::NotSupported => "Not supported".into(), SecureBoot::Unknown => "unknown".into() });
                let bl: Vec<_> = hw.bitlocker.iter().map(|v| format!("{} {}", v.mount, match v.state { BitLocker::Active => "encrypted", BitLocker::Off => "off", BitLocker::Unknown => "unknown" })).collect();
                row(ui, "BitLocker", if bl.is_empty() { "unknown".into() } else { bl.join(", ") });
                if let Some(c) = &hw.c_drive {
                    row(ui, "C: drive", format!("{} free of {}", download::human(c.free), download::human(c.size)));
                }
                let data: u64 = r.user_data.folders.iter().map(|f| f.stats.bytes).sum();
                row(ui, "Your folders", format!("{} in Documents, Desktop, Pictures, Music, Videos and Downloads", download::human(data)));
            });
        });

        // ---- Programs ----
        section(ui, "Your programs on Linux");
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(self.filter.is_none(), format!("All  {}", shown.len())).clicked() {
                self.filter = None;
            }
            for (s, label) in STATUSES {
                if ui.selectable_label(self.filter == Some(s), RichText::new(format!("{label}  {}", count(s))).color(status_color(s))).clicked() {
                    self.filter = Some(s);
                }
            }
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search…").desired_width(180.0));
        });
        ui.add_space(4.0);
        let needle = self.search.to_lowercase();
        let mut list: Vec<_> = shown.iter().filter(|p| self.filter.is_none_or(|f| p.status == f) && (needle.is_empty() || p.program.name.to_lowercase().contains(&needle))).collect();
        list.sort_by_key(|p| (p.status, p.program.name.to_lowercase()));
        card(ui, |ui| {
            {
                let w = ui.available_width();
                let (name_w, status_w) = ((w * 0.34).max(180.0), 190.0);
                for (i, p) in list.iter().take(400).enumerate() {
                    let (row, _) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::hover());
                    if i % 2 == 1 {
                        ui.painter().rect_filled(row, egui::CornerRadius::same(6), CARD_HI);
                    }
                    let cell = |x: f32, width: f32| egui::Rect::from_min_size(egui::pos2(row.left() + x, row.top()), egui::vec2(width, row.height()));
                    let mut put = |rect: egui::Rect, add: &mut dyn FnMut(&mut egui::Ui)| {
                        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(8.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        add(&mut c);
                    };
                    put(cell(0.0, name_w), &mut |ui| {
                        ui.add(egui::Label::new(RichText::new(&p.program.name).color(TEXT)).truncate());
                    });
                    put(cell(name_w, status_w), &mut |ui| {
                        ui.label(RichText::new(p.status.label()).size(13.0).color(status_color(p.status)));
                    });
                    let hint = linux_hint(p);
                    put(cell(name_w + status_w, w - name_w - status_w), &mut |ui| match &p.protondb {
                        Some(url) if hint.is_empty() => {
                            ui.hyperlink_to(RichText::new("ProtonDB reports").size(13.0), url);
                        }
                        _ => {
                            ui.add(egui::Label::new(RichText::new(&hint).size(13.0).color(MUTED)).truncate());
                        }
                    });
                }
                if list.len() > 400 {
                    muted(ui, &format!("… and {} more (search to narrow it down, or open the full report).", list.len() - 400));
                }
                if list.is_empty() {
                    muted(ui, "Nothing matches.");
                }
            }
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if secondary(ui, "Open the full report", true).clicked() {
                if let Ok(d) = appdirs::reports_dir() {
                    open_path(&d.join("latest").join("report.html"));
                }
            }
            if secondary(ui, "Scan again", true).clicked() {
                self.start(ctx, sh);
            }
        });
        next_button(ui, "Next: choose a distro  →", Page::Choose)
    }

    fn start(&mut self, ctx: &egui::Context, sh: &Shared) {
        let data_dir = sh.data_dir.clone();
        let use_winget = !self.no_winget;
        self.task.start(ctx, move |rep| {
            let (map, source) = AppMap::load(data_dir.as_deref())?;
            let r = scan::run(&map, source, &scan::ScanOptions { use_winget }, |m| rep.stage(m.trim_end_matches("...")));
            rep.stage("Writing the report");
            let dir = report::write(&r, &appdirs::reports_dir()?)?;
            Ok(dir.join("report.html"))
        });
    }
}

/// What to use on Linux, in a few words.
fn linux_hint(p: &crate::mapping::MappedProgram) -> String {
    let Some(e) = &p.entry else { return String::new() };
    match p.status {
        Status::Alternative => e.alternatives.iter().map(|a| a.name.as_str()).take(3).collect::<Vec<_>>().join(", "),
        _ => e.notes.clone().unwrap_or_default(),
    }
}

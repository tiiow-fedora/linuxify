//! Stage 5: dual-boot install. The page always shows the dry-run plan first,
//! lists every reason it won't run, and enables the button only when the typed
//! phrase matches. The actual gates are in `flow` (shared with the CLI): the
//! click re-runs the whole preparation and `flow::install_confirmed` checks the
//! fresh plan's phrase again, so a stale screen can't approve a changed plan.

use super::job::{Reporter, Task};
use super::style::*;
use super::{distro_combo, Shared};
use crate::flow::{self, InstallPrep};
use crate::install::{self, InstallState, Phase, ReplaceGate};
use crate::{appdirs, download, install_exec, iso};
use eframe::egui::{self, RichText};
use std::path::PathBuf;

pub struct InstallPage {
    distro: String,
    custom_size: bool,
    linux_gb: u64,
    check: Task<InstallPrep>,
    typed: String,
    run: Task<InstallState>,
    iso: Task<PathBuf>,
    undo_plan: Task<Vec<String>>,
    undo_typed: String,
    undo_run: Task<Vec<String>>,
    gate: Task<(ReplaceGate, Vec<String>)>,
    elevate_failed: bool,
}

impl Default for InstallPage {
    fn default() -> Self {
        InstallPage {
            distro: String::new(),
            custom_size: false,
            linux_gb: install::DEFAULT_LINUX / install::GB,
            check: Task::Idle,
            typed: String::new(),
            run: Task::Idle,
            iso: Task::Idle,
            undo_plan: Task::Idle,
            undo_typed: String::new(),
            undo_run: Task::Idle,
            gate: Task::Idle,
            elevate_failed: false,
        }
    }
}

/// What the running install is doing, and roughly how far along it is.
fn phase_text(p: &Phase) -> (&'static str, f32) {
    match p {
        Phase::Started => ("Checking the ISO and shrinking C: (this can take a few minutes)", 0.1),
        Phase::Shrunk => ("Copying the installer files to the new partition", 0.4),
        Phase::FilesCopied => ("Adding the one-time boot entry", 0.85),
        Phase::BootEntryAdded | Phase::Ready => ("Finishing", 0.95),
    }
}

impl InstallPage {
    fn busy(&self) -> bool {
        self.check.running() || self.run.running() || self.iso.running() || self.undo_plan.running() || self.undo_run.running() || self.gate.running()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, sh: &mut Shared) -> Option<super::Page> {
        self.check.tick();
        self.gate.tick();
        self.undo_plan.tick();
        if self.run.tick() | self.iso.tick() | self.undo_run.tick() {
            sh.refresh();
            // Anything changed: the old plan and phrase no longer apply.
            self.typed.clear();
            self.undo_typed.clear();
            if self.iso.ok().is_some() {
                self.start_check(ctx, sh);
            }
        }
        header(ui, "STEP 5", "Install Linux alongside Windows", "Shrinks C:, puts the distro's own installer on a small partition and restarts into it once. Windows stays.", Some(("Changes your disk", AMBER)));

        tinted(ui, AMBER, |ui| {
            ui.label(RichText::new("⚠  Experimental: read this first").font(bold(15.5)).color(AMBER));
            bullet(ui, "•", AMBER, "This has been tested only in throwaway virtual machines, never on real hardware.");
            bullet(ui, "•", AMBER, "Back up everything to a separate disk you then unplug. The copy from step 4 is not a backup.");
            bullet(ui, "•", AMBER, "If you're not comfortable fixing a PC that won't start, use step 3 and install from a USB stick instead.");
        });
        if let Some(lock) = &sh.lock {
            ui.add_space(8.0);
            tinted(ui, RED, |ui| {
                ui.label(RichText::new("🔒  This PC is locked against disk changes").font(bold(15.5)).color(RED));
                muted(ui, &format!("{} exists, so Linuxify will show the plan but never change a disk here. Delete that file only if you really want Linuxify to change this PC's disks.", lock.display()));
            });
        }

        if let Task::Running(job) = &self.run {
            let mut p = job.progress();
            if let Some(s) = install_exec::load_state() {
                let (text, done) = phase_text(&s.phase);
                (p.stage, p.fraction) = (text.into(), Some(done));
            }
            progress(ui, &p, job.started.elapsed());
            muted(ui, "Don't turn the PC off. If anything fails, Linuxify stops and “Undo” puts it back.");
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
            return None;
        }
        if let Task::Done(Ok(state)) = &self.run {
            let p = &state.plan;
            tinted(ui, ACCENT, |ui| {
                ui.label(RichText::new("✔  Ready: the next restart starts the installer").font(bold(17.0)).color(ACCENT));
                muted(ui, "Later restarts go to Windows as usual. Restart when you're ready.");
                ui.add_space(4.0);
                bullet(ui, "→", ACCENT, &install::installer_hint(self.check.ok().map(|c| c.distro.installer).unwrap_or_default(), p.disk_size));
                bullet(ui, "↶", MUTED, "Changed your mind? “Undo” below puts it back.");
                bullet(
                    ui,
                    "•",
                    MUTED,
                    &format!(
                        "After Linux is installed, the same Undo (from Windows) removes the {} GB installer partition{} and gives the space back to C:. Linux stays.",
                        p.installer_size / install::GB,
                        if p.data_size > 0 { "s" } else { "" }
                    ),
                );
            });
            ui.add_space(8.0);
        }
        if let Task::Done(Ok(lines)) = &self.undo_run {
            tinted(ui, ACCENT, |ui| {
                ui.label(RichText::new("✔  Undo finished").font(bold(15.5)).color(ACCENT));
                code(ui, &lines.iter().map(|l| format!("done: {l}")).collect::<Vec<_>>().join("\n"));
                muted(ui, "The installer partition and boot entry are gone and C: has its space back.");
            });
            ui.add_space(8.0);
        }
        if let Some(state) = sh.install_state.clone() {
            self.undo_section(ui, ctx, &state);
            self.gate_section(ui, ctx);
            return None;
        }
        if matches!(self.run, Task::Done(Ok(_))) {
            self.gate_section(ui, ctx);
            return None;
        }

        // ---- Choose ----
        let busy = self.busy();
        section(ui, "1. What to install");
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Distro").color(MUTED));
                ui.add_enabled_ui(!busy, |ui| distro_combo(ui, "install-distro", sh, true, &mut self.distro));
            });
            muted(ui, "Only distros whose official ISO Linuxify can download and verify are offered here. For the others, use a USB stick.");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_enabled(!busy, egui::Checkbox::new(&mut self.custom_size, "Choose the space for Linux myself:"));
                ui.spacing_mut().slider_width = 280.0;
                ui.add_enabled(self.custom_size && !busy, egui::Slider::new(&mut self.linux_gb, install::MIN_LINUX / install::GB..=1000).suffix(" GB"));
            });
            if !self.custom_size {
                muted(ui, &format!("Default: {} GB, or as much as fits safely.", install::DEFAULT_LINUX / install::GB));
            }
            ui.add_space(8.0);
            if primary(ui, if self.check.ok().is_some() { "Check again" } else { "Check this PC and show the plan" }, !busy && !self.distro.is_empty()).clicked() {
                self.start_check(ctx, sh);
            }
            muted(ui, "This is the dry run: it re-scans the disks, re-verifies your copied files and works out the plan. It changes nothing.");
        });

        ui.add_space(10.0);
        if let Task::Running(j) = &self.check {
            progress(ui, &j.progress(), j.started.elapsed());
        }
        if let Task::Running(j) = &self.iso {
            progress(ui, &j.progress(), j.started.elapsed());
        }
        if let Some(e) = self.iso.err() {
            error_box(ui, "The ISO couldn't be downloaded", e);
        }
        if let Some(e) = self.check.err() {
            error_box(ui, "Couldn't check this PC", e);
        }
        if let Some(e) = self.run.err() {
            error_box(ui, "The install did not go ahead", &format!("{e}\n\nIf it stopped part-way, “Undo” (shown once you check again) puts everything back."));
        }
        // The plan view may start other jobs, so it borrows the check result separately.
        let check = std::mem::take(&mut self.check);
        if let Some(prep) = check.ok() {
            self.plan_ui(ui, ctx, sh, prep, busy);
        }
        if matches!(self.check, Task::Idle) {
            self.check = check;
        }
        None
    }

    fn plan_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, sh: &Shared, prep: &InstallPrep, busy: bool) {
        // ---- Plan ----
        section(ui, "2. The plan");
        match &prep.plan {
            Ok(p) => card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Disk {}  ·  {} as a dual boot", p.disk, prep.distro.name)).font(bold(16.0)).color(TEXT));
                    if prep.estimate {
                        pill(ui, "Estimate", AMBER);
                    }
                });
                if prep.estimate {
                    muted(ui, "Without administrator rights Windows won't say exactly how far C: can shrink, so this assumes its free space.");
                }
                ui.add_space(4.0);
                for (i, s) in flow::plan_steps(p, &prep.distro.name).iter().enumerate() {
                    bullet(ui, &format!("{}.", i + 1), ACCENT, s);
                }
                ui.add_space(4.0);
                tinted(ui, BLUE, |ui| {
                    ui.label(RichText::new("In the installer, after the restart").font(bold(14.5)).color(BLUE));
                    muted(ui, &install::installer_hint(prep.distro.installer, p.disk_size));
                });
                ui.add_space(4.0);
                egui::CollapsingHeader::new(RichText::new("Exact commands").color(MUTED)).show(ui, |ui| code(ui, &flow::plan_commands(p)));
            }),
            Err(why) => tinted(ui, RED, |ui| {
                ui.label(RichText::new("Linuxify can't make a plan").font(bold(15.5)).color(RED));
                for w in why {
                    bullet(ui, "✕", RED, w);
                }
            }),
        }

        // ---- Blockers ----
        section(ui, "3. Safety checks");
        if prep.problems.is_empty() && !prep.estimate {
            tinted(ui, ACCENT, |ui| {
                ui.label(RichText::new("✔  Every check passed").font(bold(15.5)).color(ACCENT));
                if let Some(s) = &prep.backup_staging {
                    muted(ui, &format!("Your copied files were verified in {}.", s.display()));
                }
            });
        } else {
            tinted(ui, RED, |ui| {
                ui.label(RichText::new(format!("Linuxify won't continue until {} fixed:", if prep.problems.len() == 1 { "this is" } else { "these are" })).font(bold(15.5)).color(RED));
                for pr in &prep.problems {
                    bullet(ui, "✕", RED, pr);
                }
                if prep.problems.is_empty() && prep.estimate {
                    bullet(ui, "✕", RED, "the plan is only an estimate: restart Linuxify as administrator for the exact one");
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if !prep.is_admin && sh.lock.is_none() && secondary(ui, "Restart as administrator", !busy).clicked() {
                        if super::restart_as_admin("install") {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        } else {
                            self.elevate_failed = true;
                        }
                    }
                    if !prep.iso_verified && secondary(ui, &format!("Download and verify the {} ISO", prep.distro.name), !busy).clicked() {
                        self.start_iso(ctx, sh);
                    }
                });
                if self.elevate_failed {
                    muted(ui, "Windows didn't start it as administrator (the prompt was declined?).");
                }
            });
        }

        // ---- Confirm ----
        let Some(plan) = prep.runnable() else { return };
        let phrase = plan.confirm_phrase();
        section(ui, "4. Confirm");
        tinted(ui, RED, |ui| {
            ui.label(RichText::new("This changes your disk").font(bold(16.0)).color(RED));
            muted(ui, "Windows and your files stay. To go ahead, type this exactly (it names the disk and the size):");
            code(ui, &phrase);
            ui.add(egui::TextEdit::singleline(&mut self.typed).hint_text("Type the phrase here").desired_width(420.0).font(egui::TextStyle::Monospace));
            ui.add_space(6.0);
            let matches = self.typed.trim() == phrase;
            if !self.typed.is_empty() && !matches {
                ui.label(RichText::new("Doesn't match yet.").size(13.0).color(AMBER));
            }
            if danger(ui, &format!("Shrink C: and set up the {} installer", prep.distro.name), matches && !busy).clicked() {
                let (data_dir, distro, typed) = (sh.data_dir.clone(), prep.distro.id.clone(), self.typed.clone());
                let linux_gb = self.custom_size.then_some(self.linux_gb);
                self.run.start(ctx, move |rep| run_install(rep, data_dir, &distro, linux_gb, &typed));
            }
        });
    }

    fn start_check(&mut self, ctx: &egui::Context, sh: &Shared) {
        let (data_dir, distro) = (sh.data_dir.clone(), self.distro.clone());
        let linux_gb = self.custom_size.then_some(self.linux_gb);
        self.typed.clear();
        if !self.undo_run.running() {
            self.undo_run = Task::Idle;
        }
        self.check.start(ctx, move |rep| flow::prepare_install(data_dir.as_deref(), &distro, linux_gb, true, &mut |m| rep.stage(m.trim_end_matches("...")), &mut |d, t| rep.bytes(d, t, "")));
    }

    fn start_iso(&mut self, ctx: &egui::Context, sh: &Shared) {
        let (data_dir, distro) = (sh.data_dir.clone(), self.distro.clone());
        self.iso.start(ctx, move |rep| {
            let isos = iso::IsoList::load(data_dir.as_deref())?;
            let src = isos.find(&distro).ok_or("no official ISO source for this distro")?;
            rep.stage("Checking the official checksum list");
            let r = download::resolve(src)?;
            download::ensure_iso_with(&r, &appdirs::home()?.join("isos"), false, &mut rep.download_events())
        });
    }

    fn undo_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &InstallState) {
        section(ui, "Undo");
        let busy = self.busy();
        card(ui, |ui| {
            ui.label(RichText::new(format!("An install setup is in place (disk {}, step: {:?})", state.plan.disk, state.phase)).font(bold(16.0)).color(TEXT));
            muted(ui, &flow::undo_summary(state));
            muted(ui, "It does NOT remove a Linux system if you already installed one into the free space.");
            ui.add_space(6.0);
            if secondary(ui, "Show exactly what undo will run", !busy).clicked() {
                let st = state.clone();
                self.undo_plan.start(ctx, move |rep| {
                    rep.stage("Re-scanning the disk (dry run)");
                    install_exec::undo(&st, true)
                });
            }
            match &self.undo_plan {
                Task::Running(j) => progress(ui, &j.progress(), j.started.elapsed()),
                Task::Done(Ok(lines)) => code(ui, &lines.iter().map(|l| format!("would run: {l}")).collect::<Vec<_>>().join("\n")),
                Task::Done(Err(e)) => error_box(ui, "Couldn't work out the undo", e),
                Task::Idle => {}
            }
            ui.add_space(6.0);
            let phrase = flow::undo_phrase(state);
            muted(ui, "To undo, type exactly:");
            code(ui, &phrase);
            ui.add(egui::TextEdit::singleline(&mut self.undo_typed).hint_text("Type the phrase here").desired_width(420.0).font(egui::TextStyle::Monospace));
            let ok = self.undo_typed.trim() == phrase;
            if danger(ui, "Undo the install setup", ok && !busy).clicked() {
                let (st, typed) = (state.clone(), self.undo_typed.clone());
                self.undo_run.start(ctx, move |rep| {
                    rep.stage("Removing the boot entry and the installer partition, growing C: back");
                    flow::undo_confirmed(&st, &typed)
                });
            }
            match &self.undo_run {
                Task::Running(j) => progress(ui, &j.progress(), j.started.elapsed()),
                Task::Done(Err(e)) => error_box(ui, "Undo didn't run", e),
                _ => {}
            }
        });
    }

    fn gate_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(6.0);
        egui::CollapsingHeader::new(RichText::new("Later: removing Windows completely").color(MUTED)).show(ui, |ui| {
            muted(ui, "Removing Windows is done from Linux, after you've restored your files there, by remove-windows.sh. Linuxify only checks here whether it may be unlocked; it never deletes anything from Windows.");
            if secondary(ui, "Check whether it can be unlocked", !self.busy()).clicked() {
                self.gate.start(ctx, |rep| {
                    rep.stage("Verifying your staged copy (re-reading every file)");
                    Ok(flow::replace_windows_gate(|d, t| rep.bytes(d, t, "")))
                });
            }
            match &self.gate {
                Task::Running(j) => progress(ui, &j.progress(), j.started.elapsed()),
                Task::Done(Ok((_, problems))) if !problems.is_empty() => tinted(ui, RED, |ui| {
                    ui.label(RichText::new("Removing Windows stays locked because:").font(bold(15.0)).color(RED));
                    for p in problems {
                        bullet(ui, "✕", RED, p);
                    }
                }),
                Task::Done(Ok((g, _))) => tinted(ui, ACCENT, |ui| {
                    ui.label(RichText::new(format!("✔  Your staged copy verified completely ({} files)", g.staging_files_ok)).font(bold(15.0)).color(ACCENT));
                    muted(ui, "1. Boot into Linux, open the linuxify-migration folder, run:  bash restore.sh");
                    muted(ui, "2. Then:  bash remove-windows.sh  (it re-checks every restored file before it will touch Windows, shows exactly which partitions it would delete, and needs a typed confirmation).");
                }),
                Task::Done(Err(e)) => error_box(ui, "Couldn't check", e),
                Task::Idle => {}
            }
        });
    }
}

/// Re-prepares from scratch (fresh scan, fresh verification), then goes
/// through the shared gate with the phrase the user typed.
fn run_install(rep: &Reporter, data_dir: Option<PathBuf>, distro: &str, linux_gb: Option<u64>, typed: &str) -> Result<InstallState, String> {
    let prep = flow::prepare_install(data_dir.as_deref(), distro, linux_gb, false, &mut |m| rep.stage(m.trim_end_matches("...")), &mut |d, t| rep.bytes(d, t, ""))?;
    flow::check_install_gates(&prep, typed).map_err(|e| format!("{e}. The PC changed since the check, or the plan did; check again and read the new plan."))?;
    rep.stage("Changing the disk");
    flow::install_confirmed(&prep, typed)
}

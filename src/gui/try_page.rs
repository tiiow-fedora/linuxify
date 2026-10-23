//! Stage 3: download + verify the official ISO and boot it in a VM.

use super::job::{Reporter, Task};
use super::style::*;
use super::{distro_combo, next_button, Page, Shared};
use crate::chooser::{DistroList, SecureBootSupport};
use crate::{appdirs, download, flow, iso, vm};
use eframe::egui::{self, RichText};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq)]
enum Action {
    /// Dry run: what would be downloaded and run.
    Plan,
    Start,
    DownloadOnly,
    Remove,
}

pub enum Outcome {
    Plan(Vec<String>),
    Message(String),
}

pub struct TryPage {
    distro: String,
    use_disk: bool,
    disk_gb: u32,
    iso_file: Option<PathBuf>,
    backends: Option<Vec<vm::Detected>>,
    task: Task<Outcome>,
    last: Option<Action>,
    confirm_remove: bool,
}

impl Default for TryPage {
    fn default() -> Self {
        TryPage { distro: String::new(), use_disk: false, disk_gb: 25, iso_file: None, backends: None, task: Task::Idle, last: None, confirm_remove: false }
    }
}

impl TryPage {
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, sh: &mut Shared) -> Option<Page> {
        if self.task.tick() {
            sh.refresh();
        }
        header(ui, "STEP 3", "Try it in a virtual machine", "Linux runs in a window on Windows. Nothing is installed and your real disks aren't touched.", Some(("Zero risk", ACCENT)));
        let is_admin = sh.scan.as_ref().is_some_and(|r| r.hardware.is_admin);
        let backends = self.backends.get_or_insert_with(|| vm::detect(is_admin)).clone();
        let Ok(list) = &sh.distros else { return None };
        let busy = self.task.running();

        // ---- Distro + options ----
        card(ui, |ui| {
            ui.label(RichText::new("Which distro?").font(bold(16.0)).color(TEXT));
            ui.add_enabled_ui(!busy, |ui| distro_combo(ui, "try-distro", sh, false, &mut self.distro));
            let automatic = sh.isos.as_ref().is_some_and(|i| i.find(&self.distro).is_some());
            let d = list.distros.iter().find(|d| d.id == self.distro);
            if automatic {
                bullet(ui, "✔", ACCENT, "Linuxify downloads the official ISO and checks its SHA-256 against the distro's own checksum list.");
            } else if let Some(d) = d {
                bullet(ui, "ⓘ", AMBER, &format!("Linuxify can't download {} automatically yet. Download the ISO from its website, then pick the file here.", d.name));
                ui.horizontal(|ui| {
                    ui.hyperlink_to(&d.homepage, &d.homepage);
                });
            }
            ui.horizontal(|ui| {
                if secondary(ui, if automatic { "Use an ISO I already have…" } else { "Pick the ISO file…" }, !busy).clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Disk image", &["iso"]).pick_file() {
                        self.iso_file = Some(p);
                    }
                }
                if let Some(p) = &self.iso_file {
                    ui.label(RichText::new(p.display().to_string()).monospace().color(MUTED));
                    if ui.small_button("✕").clicked() {
                        self.iso_file = None;
                    }
                }
            });
            if let Some(d) = d {
                if d.secure_boot != SecureBootSupport::Supported {
                    muted(ui, &format!("Note: the VM runs with Secure Boot off, so it doesn't show how {} behaves with Secure Boot on.", d.name));
                }
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.use_disk, "Also give the VM an empty virtual disk, to test the installer:");
                ui.add_enabled(self.use_disk, egui::DragValue::new(&mut self.disk_gb).range(10..=500).suffix(" GB"));
            });
        });

        // ---- VM programs ----
        section(ui, "VM program");
        card(ui, |ui| {
            if backends.is_empty() {
                ui.label(RichText::new("No VM program found").font(bold(15.5)).color(AMBER));
                muted(ui, "VirtualBox is free and the easiest. Install it from a terminal with the command below (or from virtualbox.org), then click “Check again”.");
                code(ui, "winget install Oracle.VirtualBox");
            }
            for b in &backends {
                match &b.unusable {
                    None => bullet(ui, "✔", ACCENT, &format!("{}  ·  {}", b.backend.label(), b.exe.display())),
                    Some(u) => bullet(ui, "✕", FAINT, &format!("{}: {u}", b.backend.label())),
                }
            }
            if secondary(ui, "Check again", !busy).clicked() {
                self.backends = None;
            }
        });

        // ---- Actions ----
        ui.add_space(12.0);
        let usable = flow::pick_backend(&backends, None).is_ok();
        ui.horizontal_wrapped(|ui| {
            if primary(ui, "Start the trial", !busy && usable).clicked() {
                self.run(ctx, sh, Action::Start);
            }
            if secondary(ui, "Show what will happen", !busy).clicked() {
                self.run(ctx, sh, Action::Plan);
            }
            if secondary(ui, "Only download and verify the ISO", !busy && sh.isos.as_ref().is_some_and(|i| i.find(&self.distro).is_some())).clicked() {
                self.run(ctx, sh, Action::DownloadOnly);
            }
            let has_vbox = backends.iter().any(|b| b.backend == vm::Backend::VirtualBox);
            if has_vbox && secondary(ui, if self.confirm_remove { "Click again to delete the trial VM" } else { "Delete the trial VM" }, !busy).clicked() {
                if self.confirm_remove {
                    self.confirm_remove = false;
                    self.run(ctx, sh, Action::Remove);
                } else {
                    self.confirm_remove = true;
                }
            }
        });
        ui.add_space(10.0);
        match &self.task {
            Task::Running(job) => progress(ui, &job.progress(), job.started.elapsed()),
            Task::Done(Err(e)) => error_box(ui, "That didn't work", e),
            Task::Done(Ok(Outcome::Plan(lines))) => card(ui, |ui| {
                ui.label(RichText::new("Dry run: nothing was downloaded or started").font(bold(15.5)).color(TEXT));
                code(ui, &lines.join("\n"));
            }),
            Task::Done(Ok(Outcome::Message(m))) => tinted(ui, ACCENT, |ui| {
                ui.label(RichText::new("Done").font(bold(15.5)).color(ACCENT));
                ui.add(egui::Label::new(RichText::new(m).color(TEXT)).wrap());
            }),
            Task::Idle => {}
        }
        next_button(ui, "Next: copy your files  →", Page::Migrate)
    }

    fn run(&mut self, ctx: &egui::Context, sh: &Shared, action: Action) {
        let (data_dir, distro_id, iso_file) = (sh.data_dir.clone(), self.distro.clone(), self.iso_file.clone());
        let disk = self.use_disk.then_some(self.disk_gb);
        let backends = self.backends.clone().unwrap_or_default();
        self.last = Some(action);
        self.task.start(ctx, move |rep| trial(rep, data_dir, &distro_id, iso_file, disk, &backends, action));
    }
}

/// The same steps as `linuxify try`, reporting to the page.
fn trial(rep: &Reporter, data_dir: Option<PathBuf>, distro_id: &str, iso_file: Option<PathBuf>, disk: Option<u32>, backends: &[vm::Detected], action: Action) -> Result<Outcome, String> {
    let (distros, _) = DistroList::load(data_dir.as_deref())?;
    let isos = iso::IsoList::load(data_dir.as_deref())?;
    let distro = flow::find_distro(&distros, distro_id)?;
    let iso_dir = appdirs::home()?.join("isos");
    let dry = action == Action::Plan;

    if action == Action::Remove {
        let b = backends.iter().find(|b| b.backend == vm::Backend::VirtualBox).ok_or("deleting the trial VM is automated for VirtualBox only")?;
        if !vm::vbox_vm_exists(&b.exe, &distro.id) {
            return Ok(Outcome::Message(format!("There is no trial VM for {}.", distro.name)));
        }
        rep.stage(format!("Deleting '{}'", vm::vm_name(&distro.id)));
        vm::execute(&vm::plan_virtualbox_remove(&b.exe, &distro.id))?;
        return Ok(Outcome::Message(format!("Removed the trial VM for {}. The downloaded ISO was kept in {}.", distro.name, iso_dir.display())));
    }

    let mut lines = Vec::new();
    let resolved = match isos.find(&distro.id) {
        Some(s) => {
            rep.stage("Checking the official checksum list");
            let r = download::resolve(s)?;
            lines.push(format!("Latest ISO:    {}\nSHA-256:       {}\nChecksum from: {}", r.file_name, r.sha256, r.checksum_url));
            Some(r)
        }
        None => None,
    };
    let mut on = rep.download_events();
    let mut warning = String::new();
    let iso_path = match (&iso_file, &resolved) {
        (Some(local), Some(r)) if dry => {
            lines.push(format!("\nWould check {} against the official SHA-256 above.", local.display()));
            local.clone()
        }
        (Some(local), Some(r)) => download::verify_local_with(local, r, &mut on)?,
        (Some(local), None) => {
            warning = format!(
                "\n\nWarning: Linuxify has no official checksum source for {} yet, so it could not verify {}. Compare it yourself with the checksum on {} before trusting it.",
                distro.name,
                local.display(),
                distro.homepage
            );
            local.clone()
        }
        (None, Some(r)) if dry => {
            lines.push(format!("\nWould download\n  {}\n  to {}\nand refuse to use it unless its SHA-256 matches.", r.url, iso_dir.join(&r.file_name).display()));
            iso_dir.join(&r.file_name)
        }
        (None, Some(r)) => download::ensure_iso_with(r, &iso_dir, false, &mut on)?,
        (None, None) => return Err(format!("Linuxify can't download {} automatically yet. Download the ISO from {} and pick the file with “Pick the ISO file…”.", distro.name, distro.homepage)),
    };
    drop(on);
    if action == Action::DownloadOnly {
        return Ok(Outcome::Message(format!("Ready: {} (verified against the official checksum).", iso_path.display())));
    }

    let chosen = flow::pick_backend(backends, None)?;
    let spec = flow::trial_spec(&distro.id, iso_path, disk)?;
    let steps = flow::trial_steps(chosen, &spec, dry);
    if dry {
        let disk = disk.map(|g| format!(", {g} GB virtual disk")).unwrap_or_default();
        lines.push(format!("\nWould run these {} commands ({} MB RAM, {} CPUs{disk}):", chosen.backend.label(), spec.memory_mb, spec.cpus));
        for s in &steps {
            lines.push(format!("  {}\n      ({})", s.display(), s.what));
        }
        return Ok(Outcome::Plan(lines));
    }
    std::fs::create_dir_all(&spec.vm_dir).map_err(|e| e.to_string())?;
    rep.stage(format!("Starting the VM in {} ({} MB RAM, {} CPUs)", chosen.backend.label(), spec.memory_mb, spec.cpus));
    vm::execute(&steps)?;
    Ok(Outcome::Message(format!(
        "The {} live session should now be open in a window. Nothing is installed anywhere; just power the VM off when you're done.{warning}",
        distro.name
    )))
}

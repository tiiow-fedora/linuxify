//! Stage 4: copy (never move) the user's files into a Linux-ready staging folder.

use super::job::{Reporter, Task};
use super::style::*;
use super::{next_button, open_path, Page, Shared};
use crate::{download, flow, migrate, scan};
use eframe::egui::{self, RichText};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct PlanSummary {
    staging: PathBuf,
    per_source: Vec<(String, u64, u64)>,
    total_files: u64,
    total_bytes: u64,
    skipped: Vec<(String, usize)>,
    free: Option<u64>,
    reusable: u64,
    on_c: bool,
    /// Why the copy would be refused (FAT32 limit, not enough space).
    refusal: Option<String>,
}

pub struct CopySummary {
    staging: PathBuf,
    files: u64,
    bytes: u64,
    newly_copied: u64,
    reused: u64,
    failed: Vec<migrate::Skipped>,
    apps: usize,
    verify: Option<migrate::VerifyReport>,
}

#[derive(Default)]
pub struct MigratePage {
    target: String,
    plan: Task<PlanSummary>,
    copy: Task<CopySummary>,
    verify: Task<migrate::VerifyReport>,
}

fn sources() -> Vec<migrate::SourceItem> {
    migrate::default_sources(&scan::userdata::scan())
}

fn make_plan(rep: &Reporter, target: &Path) -> Result<(PathBuf, Vec<migrate::SourceItem>, migrate::Plan), String> {
    if !target.is_dir() {
        return Err(format!("{} is not an existing folder. Pick a folder on another drive, a USB disk or a NAS share.", target.display()));
    }
    let staging = target.join(migrate::STAGING_DIR);
    rep.stage("Finding your files");
    let sources = sources();
    migrate::check_target(&staging, &sources)?;
    rep.stage("Planning the copy (reading file sizes only)");
    let plan = migrate::plan(&sources);
    Ok((staging, sources, plan))
}

impl MigratePage {
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, sh: &mut Shared) -> Option<Page> {
        self.plan.tick();
        if self.copy.tick() {
            sh.refresh();
        }
        self.verify.tick();
        header(
            ui,
            "STEP 4",
            "Copy your files for Linux",
            "Copies your documents, pictures, browser bookmarks, SSH keys and more into a folder Linux can restore from. Your originals are only read.",
            Some(("Copies, never moves", ACCENT)),
        );
        let busy = self.plan.running() || self.copy.running() || self.verify.running();
        if self.target.is_empty() {
            if let Some((staging, _)) = &sh.migration {
                if let Some(parent) = staging.parent() {
                    self.target = parent.display().to_string();
                }
            }
        }

        card(ui, |ui| {
            ui.label(RichText::new("Where should the copy go?").font(bold(16.0)).color(TEXT));
            muted(ui, "Pick a folder on another drive, a USB disk or a NAS share. A “linuxify-migration” folder is created inside it. Use a different drive than C: if you plan to install Linux on this PC.");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_enabled(!busy, egui::TextEdit::singleline(&mut self.target).hint_text(r"e.g. E:\").desired_width(360.0));
                if secondary(ui, "Browse…", !busy).clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.target = p.display().to_string();
                        self.plan = Task::Idle;
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let ok = !self.target.trim().is_empty() && !busy;
                if secondary(ui, "Check what will be copied", ok).clicked() {
                    let target = PathBuf::from(self.target.trim());
                    self.copy = Task::Idle;
                    self.plan.start(ctx, move |rep| summarize(rep, &target));
                }
                let can_copy = ok && self.plan.ok().is_some_and(|p| p.refusal.is_none());
                if primary(ui, "Copy my files", can_copy).clicked() {
                    let target = PathBuf::from(self.target.trim());
                    self.copy.start(ctx, move |rep| copy(rep, &target));
                }
                if let Some((staging, _)) = sh.migration.clone() {
                    if secondary(ui, "Verify the existing copy", !busy).clicked() {
                        self.verify.start(ctx, move |rep| {
                            rep.stage("Re-reading every file against manifest.json");
                            migrate::verify(&staging, |d, t| rep.bytes(d, t, ""))
                        });
                    }
                }
            });
            if !self.plan.ok().is_some() && !busy {
                muted(ui, "Check first: it shows the totals and anything that won't be copied. Nothing is copied until you click “Copy my files”.");
            }
        });

        ui.add_space(10.0);
        if let Task::Running(j) = &self.plan {
            progress(ui, &j.progress(), j.started.elapsed());
        }
        if let Some(e) = self.plan.err() {
            error_box(ui, "Can't copy there", e);
        }
        if let (Some(p), false) = (self.plan.ok(), self.copy.running() || self.copy.ok().is_some()) {
            plan_view(ui, p);
        }
        match &self.copy {
            Task::Running(j) => progress(ui, &j.progress(), j.started.elapsed()),
            Task::Done(Err(e)) => error_box(ui, "The copy stopped", &format!("{e}\n\nRun it again: finished files are not copied twice.")),
            Task::Done(Ok(c)) => copy_view(ui, c),
            Task::Idle => {}
        }
        match &self.verify {
            Task::Running(j) => progress(ui, &j.progress(), j.started.elapsed()),
            Task::Done(Err(e)) => error_box(ui, "Verification couldn't run", e),
            Task::Done(Ok(r)) => verify_view(ui, r),
            Task::Idle => {}
        }
        next_button(ui, "Next: install alongside Windows  →", Page::Install)
    }
}

fn summarize(rep: &Reporter, target: &Path) -> Result<PlanSummary, String> {
    let (staging, _, plan) = make_plan(rep, target)?;
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for s in &plan.skipped {
        *reasons.entry(s.reason.clone()).or_default() += 1;
    }
    let free = fs2::available_space(target).ok();
    let total = plan.total_bytes();
    let reusable = flow::reusable_bytes(&plan, &staging);
    let mut refusal = flow::check_target_filesystem(target, &plan).err();
    if let (None, Some(f)) = (&refusal, free) {
        if total - reusable + 512 * 1024 * 1024 > f {
            refusal = Some(format!("not enough space on the target: need {} more, {} free", download::human(total - reusable), download::human(f)));
        }
    }
    Ok(PlanSummary {
        per_source: plan.per_source.iter().map(|(l, (f, b))| (l.clone(), *f, *b)).collect(),
        total_files: plan.files.len() as u64,
        total_bytes: total,
        skipped: reasons.into_iter().collect(),
        free,
        reusable,
        on_c: target.to_string_lossy().to_ascii_uppercase().starts_with("C:"),
        refusal,
        staging,
    })
}

/// The same steps as `linuxify migrate --target`, with progress.
fn copy(rep: &Reporter, target: &Path) -> Result<CopySummary, String> {
    let (staging, _, plan) = make_plan(rep, target)?;
    flow::check_target_filesystem(target, &plan)?;
    let total = plan.total_bytes();
    let reusable = flow::reusable_bytes(&plan, &staging);
    if let Ok(f) = fs2::available_space(target) {
        if total - reusable + 512 * 1024 * 1024 > f {
            return Err(format!("not enough space on the target: need {} more, {} free", download::human(total - reusable), download::human(f)));
        }
    }
    rep.stage("Copying (your originals are only read, never changed)");
    let mut last = std::time::Instant::now();
    let out = migrate::copy_all(&plan, &staging, None, |done, total, rel| {
        if last.elapsed().as_millis() > 100 {
            let name = rel.rsplit('/').next().unwrap_or(rel);
            rep.bytes(done, total, &format!("  ·  {name}"));
            last = std::time::Instant::now();
        }
    })?;
    rep.stage("Writing the manifest, restore.sh and apps.txt");
    let (manifest, apps) = flow::finish_staging(&staging, &out, &plan.skipped)?;
    rep.stage("Verifying the copy against the manifest (re-reading every file)");
    let verify = Some(migrate::verify(&staging, |d, t| rep.bytes(d, t, ""))?);
    Ok(CopySummary { staging, files: manifest.total_files, bytes: manifest.total_bytes, newly_copied: out.newly_copied, reused: out.reused, failed: out.failed, apps, verify })
}

fn count(n: u64) -> String {
    if n == 1 { "1 file".into() } else { format!("{n} files") }
}

/// Like `download::human`, but small folders show as KB instead of "0 MB".
fn size(bytes: u64) -> String {
    match bytes {
        0 => "0 MB".into(),
        b if b < 1 << 20 => format!("{} KB", b.div_ceil(1024)),
        b => download::human(b),
    }
}

fn plan_view(ui: &mut egui::Ui, p: &PlanSummary) {
    section(ui, "What will be copied");
    card(ui, |ui| {
        egui::Grid::new("plan").num_columns(3).spacing([30.0, 6.0]).striped(true).show(ui, |ui| {
            for (label, files, bytes) in &p.per_source {
                ui.label(RichText::new(label).color(TEXT));
                ui.label(RichText::new(count(*files)).color(MUTED));
                ui.label(RichText::new(size(*bytes)).color(MUTED));
                ui.end_row();
            }
            ui.label(RichText::new("Total").font(bold(15.0)).color(TEXT));
            ui.label(RichText::new(count(p.total_files)).font(bold(15.0)).color(TEXT));
            ui.label(RichText::new(size(p.total_bytes)).font(bold(15.0)).color(TEXT));
            ui.end_row();
        });
        ui.add_space(6.0);
        if let Some(f) = p.free {
            muted(ui, &format!("Free space on the target: {}.{}", download::human(f), if p.reusable > 0 { format!(" {} is already copied from an earlier run and will be reused.", size(p.reusable)) } else { String::new() }));
        }
        muted(ui, &format!("Into: {}", p.staging.display()));
    });
    if !p.skipped.is_empty() {
        ui.add_space(8.0);
        tinted(ui, BLUE, |ui| {
            ui.label(RichText::new(format!("Won't be copied ({} items)", p.skipped.iter().map(|s| s.1).sum::<usize>())).font(bold(15.0)).color(BLUE));
            for (reason, n) in &p.skipped {
                bullet(ui, &format!("{n:>5}"), MUTED, reason);
            }
        });
    }
    if p.on_c {
        ui.add_space(8.0);
        tinted(ui, AMBER, |ui| muted(ui, "The target is on C:, the Windows drive. Linux can read it in a dual boot only if BitLocker and Fast Startup are off, and the install step needs the copy on another drive."));
    }
    if let Some(r) = &p.refusal {
        ui.add_space(8.0);
        error_box(ui, "Can't copy there", r);
    }
}

fn copy_view(ui: &mut egui::Ui, c: &CopySummary) {
    let verified = c.verify.as_ref().is_some_and(|v| v.missing.is_empty() && v.mismatched.is_empty());
    tinted(ui, if verified { ACCENT } else { RED }, |ui| {
        ui.label(RichText::new(if verified { "✔  Copied and verified" } else { "The copy doesn't match its manifest" }).font(bold(17.0)).color(if verified { ACCENT } else { RED }));
        muted(ui, &format!("{} files ({}) in {}. {} copied now, {} reused from an earlier run. {} apps listed in apps.txt.", c.files, download::human(c.bytes), c.staging.display(), c.newly_copied, c.reused, c.apps));
        if let Some(v) = &c.verify {
            if !verified {
                bullet(ui, "✕", RED, &format!("{} missing, {} different from the manifest. Run the copy again to re-copy them.", v.missing.len(), v.mismatched.len()));
            }
        }
        ui.add_space(4.0);
        ui.label(RichText::new("On Linux, open that folder in a terminal and run:").color(TEXT));
        code(ui, "bash restore.sh --dry-run\nbash restore.sh");
        if secondary(ui, "Open the folder", true).clicked() {
            open_path(&c.staging);
        }
    });
    if !c.failed.is_empty() {
        ui.add_space(8.0);
        tinted(ui, AMBER, |ui| {
            ui.label(RichText::new(format!("{} files could not be copied (listed in flagged.txt)", c.failed.len())).font(bold(15.0)).color(AMBER));
            for f in c.failed.iter().take(5) {
                bullet(ui, "•", AMBER, &format!("{}  ({})", f.from, f.reason));
            }
            muted(ui, "Close the programs using them and copy again: finished files are not copied twice.");
        });
    }
}

fn verify_view(ui: &mut egui::Ui, r: &migrate::VerifyReport) {
    let ok = r.missing.is_empty() && r.mismatched.is_empty();
    ui.add_space(8.0);
    tinted(ui, if ok { ACCENT } else { RED }, |ui| {
        ui.label(RichText::new(if ok { format!("✔  {} files match the manifest", r.ok) } else { "Verification FAILED".into() }).font(bold(16.0)).color(if ok { ACCENT } else { RED }));
        for (label, list) in [("missing", &r.missing), ("different from the manifest", &r.mismatched), ("not in the manifest (extra)", &r.extra)] {
            if !list.is_empty() {
                bullet(ui, "•", if label.starts_with("not") { MUTED } else { RED }, &format!("{} {label}, e.g. {}", list.len(), list.iter().take(3).cloned().collect::<Vec<_>>().join(", ")));
            }
        }
    });
}

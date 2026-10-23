//! The Linuxify window: the same five stages as the command line, one page
//! each, for people who'd rather click than type. Every page calls the same
//! library code as the CLI (most of it through `flow`), so the safety rules
//! are identical: stages 1-4 never change a disk, and the install shows its
//! dry-run plan first, lists every blocker and needs the typed phrase.

mod choose_page;
mod install_page;
mod job;
mod migrate_page;
mod scan_page;
mod style;
mod try_page;

use crate::chooser::DistroList;
use crate::install::InstallState;
use crate::{appdirs, flow, install_exec, iso, scan};
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::path::{Path, PathBuf};
use style::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Welcome,
    Scan,
    Choose,
    Try,
    Migrate,
    Install,
}

impl Page {
    fn parse(s: &str) -> Option<Page> {
        Some(match s {
            "welcome" => Page::Welcome,
            "scan" => Page::Scan,
            "choose" => Page::Choose,
            "try" => Page::Try,
            "migrate" => Page::Migrate,
            "install" => Page::Install,
            _ => return None,
        })
    }
}

const STEPS: [(Page, &str, &str, &str); 5] = [
    (Page::Scan, "1", "Scan this PC", "Read-only"),
    (Page::Choose, "2", "Choose a distro", "A short quiz"),
    (Page::Try, "3", "Try it", "In a virtual machine"),
    (Page::Migrate, "4", "Copy your files", "Copies, never moves"),
    (Page::Install, "5", "Install", "Changes your disk"),
];

/// What several pages need to know; reloaded after each step finishes.
pub struct Shared {
    pub data_dir: Option<PathBuf>,
    pub distros: Result<DistroList, String>,
    pub isos: Option<iso::IsoList>,
    pub scan: Option<scan::ScanReport>,
    pub top_pick: Option<String>,
    pub migration: Option<(PathBuf, u64)>,
    pub install_state: Option<InstallState>,
    pub lock: Option<PathBuf>,
}

impl Shared {
    fn load(data_dir: Option<PathBuf>) -> Shared {
        let distros = DistroList::load(data_dir.as_deref()).map(|(l, _)| l);
        let isos = iso::IsoList::load(data_dir.as_deref()).ok();
        let mut s = Shared { data_dir, distros, isos, scan: None, top_pick: None, migration: None, install_state: None, lock: None };
        s.refresh();
        s
    }

    /// Re-reads the saved results of every stage.
    pub fn refresh(&mut self) {
        self.scan = flow::latest_scan();
        self.top_pick = flow::top_pick();
        self.migration = flow::recorded_migration();
        self.install_state = install_exec::load_state();
        self.lock = flow::install_lock();
    }

    fn done(&self, page: Page) -> bool {
        match page {
            Page::Welcome => false,
            Page::Scan => self.scan.is_some(),
            Page::Choose => self.top_pick.is_some(),
            Page::Try => self.any_iso_verified(),
            Page::Migrate => self.migration.is_some(),
            Page::Install => self.install_state.as_ref().is_some_and(|s| s.phase == crate::install::Phase::Ready),
        }
    }

    fn any_iso_verified(&self) -> bool {
        let Ok(dir) = appdirs::home().map(|h| h.join("isos")) else { return false };
        std::fs::read_dir(dir).is_ok_and(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x == "verified")))
    }

    /// Distro ids to offer, top pick first.
    pub fn distro_ids(&self, only_downloadable: bool) -> Vec<(String, String)> {
        let Ok(list) = &self.distros else { return vec![] };
        let mut v: Vec<(String, String)> = list
            .distros
            .iter()
            .filter(|d| !only_downloadable || self.isos.as_ref().is_some_and(|i| i.find(&d.id).is_some()))
            .map(|d| (d.id.clone(), d.name.clone()))
            .collect();
        if let Some(top) = &self.top_pick {
            if let Some(i) = v.iter().position(|(id, _)| id == top) {
                let t = v.remove(i);
                v.insert(0, t);
            }
        }
        v
    }

    pub fn distro_name(&self, id: &str) -> String {
        self.distros.as_ref().ok().and_then(|l| l.distros.iter().find(|d| d.id == id)).map(|d| d.name.clone()).unwrap_or_else(|| id.to_string())
    }
}

/// Opens a file or folder with its default program (report pages open in the browser).
pub fn open_path(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

/// A distro picker shared by the Try and Install pages.
pub fn distro_combo(ui: &mut egui::Ui, id: &str, sh: &Shared, only_downloadable: bool, selected: &mut String) {
    let ids = sh.distro_ids(only_downloadable);
    if selected.is_empty() || !ids.iter().any(|(i, _)| i == selected) {
        if let Some((first, _)) = ids.first() {
            *selected = first.clone();
        }
    }
    let label = |id: &str, name: &str| if sh.top_pick.as_deref() == Some(id) { format!("{name}   ★ your top pick") } else { name.to_string() };
    let current = ids.iter().find(|(i, _)| i == selected).map(|(i, n)| label(i, n)).unwrap_or_default();
    egui::ComboBox::from_id_salt(id).width(320.0).selected_text(current).height(420.0).show_ui(ui, |ui| {
        for (i, n) in &ids {
            ui.selectable_value(selected, i.clone(), label(i, n));
        }
    });
}

pub struct App {
    page: Page,
    shared: Shared,
    scan: scan_page::ScanPage,
    choose: choose_page::ChoosePage,
    try_page: try_page::TryPage,
    migrate: migrate_page::MigratePage,
    install: install_page::InstallPage,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, data_dir: Option<PathBuf>, page: Option<Page>) -> App {
        install_fonts(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx);
        let shared = Shared::load(data_dir);
        let page = page.unwrap_or(if shared.scan.is_some() { Page::Scan } else { Page::Welcome });
        App {
            choose: choose_page::ChoosePage::new(&shared),
            page,
            shared,
            scan: scan_page::ScanPage::default(),
            try_page: try_page::TryPage::default(),
            migrate: migrate_page::MigratePage::default(),
            install: install_page::InstallPage::default(),
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(22.0);
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            logo(ui, 34.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("Linuxify").font(bold(21.0)).color(TEXT));
                ui.label(RichText::new("Move to Linux, safely").size(12.5).color(MUTED));
            });
        });
        ui.add_space(26.0);
        if nav_row(ui, self.page == Page::Welcome, "⌂", "Welcome", "How this works", false, None) {
            self.page = Page::Welcome;
        }
        ui.add_space(6.0);
        for (page, num, title, sub) in STEPS {
            let warn = page == Page::Install;
            if nav_row(ui, self.page == page, num, title, sub, self.shared.done(page), warn.then_some(AMBER)) {
                self.page = page;
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new(format!("v{}  ·  MIT licence", env!("CARGO_PKG_VERSION"))).size(12.0).color(FAINT));
            });
            if self.shared.lock.is_some() {
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label(RichText::new("🔒 Disk changes locked on this PC").size(12.5).color(AMBER));
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("Steps 1–4 never change your disks.").size(12.5).color(MUTED));
            });
        });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        header(ui, "WELCOME", "Move from Windows to Linux, at your own pace", "Five steps. Each one is useful on its own, and you can stop after any of them.", None);
        let cols = if ui.available_width() > 760.0 { 3 } else { 2 };
        let items: [(Page, &str, &str, &str, Color32); 5] = [
            (Page::Scan, "1  Scan this PC", "Finds your programs, files and hardware, and tells you what works on Linux.", "Read-only", ACCENT),
            (Page::Choose, "2  Choose a distro", "Answer a few plain questions and get three Linux versions that suit you.", "Read-only", ACCENT),
            (Page::Try, "3  Try it", "Download the official ISO, check it, and run Linux in a virtual machine.", "Zero risk", ACCENT),
            (Page::Migrate, "4  Copy your files", "Copies your files to another drive in a Linux-ready layout, with checksums.", "Copies, never moves", ACCENT),
            (Page::Install, "5  Install alongside Windows", "Shrinks C: and starts the Linux installer. Shows the full plan first.", "Changes your disk", AMBER),
        ];
        for row in items.chunks(cols) {
            ui.columns(cols, |columns| {
                for (col, (page, title, text, risk, color)) in columns.iter_mut().zip(row) {
                    let r = egui::Frame::new()
                        .fill(CARD)
                        .stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(egui::Margin::same(16))
                        .show(col, |ui| {
                            ui.set_width(ui.available_width());
                            ui.set_min_height(112.0);
                            // Columns justify their text; cards read better ragged-right.
                            ui.vertical(|ui| {
                                ui.label(RichText::new(*title).font(bold(16.5)).color(TEXT));
                                ui.add_space(2.0);
                                muted(ui, text);
                                ui.add_space(4.0);
                                ui.horizontal(|ui| pill(ui, risk, *color));
                            });
                        })
                        .response
                        .interact(Sense::click());
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.page = *page;
                    }
                }
            });
            ui.add_space(12.0);
        }
        tinted(ui, AMBER, |ui| {
            ui.label(RichText::new("Before step 5: back up").font(bold(16.0)).color(AMBER));
            ui.add_space(2.0);
            bullet(ui, "•", AMBER, "Installing has only been tested in throwaway virtual machines, never on real hardware.");
            bullet(ui, "•", AMBER, "Back up everything to a separate disk you then unplug. Linuxify's copy (step 4) is not a backup.");
            bullet(ui, "•", AMBER, "Step 5 always shows its plan first, refuses while anything is unsafe, and needs you to type a confirmation that names the disk. It can be undone until you install Linux.");
        });
        ui.add_space(16.0);
        if primary(ui, "Get started  →", true).clicked() {
            self.page = Page::Scan;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let window = ui.clip_rect();
        const SIDEBAR_W: f32 = 250.0;
        egui::Panel::left("steps")
            .resizable(false)
            .exact_size(SIDEBAR_W)
            .frame(egui::Frame::new().fill(SIDEBAR).stroke(Stroke::new(1.0, BORDER)))
            .show(ui, |ui| self.sidebar(ui));
        // The page area is everything right of the sidebar, placed explicitly.
        let area = Rect::from_min_max(Pos2::new(window.left() + SIDEBAR_W, window.top()), window.max);
        ui.painter().rect_filled(area, CornerRadius::ZERO, BG);
        let inner = Rect::from_min_max(area.min + Vec2::new(34.0, 20.0), area.max - Vec2::new(10.0, 8.0));
        let mut page_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
        page_ui.shrink_clip_rect(area);
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(self.page as u8).show(&mut page_ui, |ui| {
            // Leave room for the scroll bar on the right.
            ui.set_max_width((ui.available_width() - 16.0).min(980.0));
            let next = match self.page {
                Page::Welcome => {
                    self.welcome(ui);
                    None
                }
                Page::Scan => self.scan.ui(ui, &ctx, &mut self.shared),
                Page::Choose => self.choose.ui(ui, &ctx, &mut self.shared),
                Page::Try => self.try_page.ui(ui, &ctx, &mut self.shared),
                Page::Migrate => self.migrate.ui(ui, &ctx, &mut self.shared),
                Page::Install => self.install.ui(ui, &ctx, &mut self.shared),
            };
            ui.add_space(24.0);
            if let Some(p) = next {
                self.page = p;
            }
        });
    }
}

/// "Next: …" button at the bottom of a page.
pub fn next_button(ui: &mut egui::Ui, text: &str, page: Page) -> Option<Page> {
    ui.add_space(14.0);
    secondary(ui, text, true).clicked().then_some(page)
}

fn nav_row(ui: &mut egui::Ui, selected: bool, num: &str, title: &str, sub: &str, done: bool, risk: Option<Color32>) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::click());
    let row = Rect::from_min_max(Pos2::new(rect.left() + 12.0, rect.top()), Pos2::new(rect.right() - 12.0, rect.bottom()));
    let p = ui.painter();
    if selected {
        p.rect_filled(row, CornerRadius::same(10), CARD_HI);
        p.rect_filled(Rect::from_min_size(Pos2::new(row.left(), row.top() + 12.0), Vec2::new(3.0, row.height() - 24.0)), CornerRadius::same(2), ACCENT);
    } else if resp.hovered() {
        p.rect_filled(row, CornerRadius::same(10), CARD);
    }
    let c = Pos2::new(row.left() + 26.0, row.center().y);
    if done {
        p.circle_filled(c, 13.0, ACCENT);
        p.text(c, egui::Align2::CENTER_CENTER, "✔", egui::FontId::proportional(13.0), ON_ACCENT);
    } else {
        p.circle_stroke(c, 12.5, Stroke::new(1.5, if selected { ACCENT } else { BORDER }));
        p.text(c, egui::Align2::CENTER_CENTER, num, bold(14.0), if selected { TEXT } else { MUTED });
    }
    let x = row.left() + 50.0;
    p.text(Pos2::new(x, row.center().y - 9.0), egui::Align2::LEFT_CENTER, title, bold(15.0), if selected { TEXT } else { Color32::from_rgb(205, 210, 220) });
    p.text(Pos2::new(x, row.center().y + 10.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(12.5), risk.unwrap_or(FAINT));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The Linuxify mark: a rounded green square with a terminal prompt.
fn logo(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same((size * 0.28) as u8), ACCENT);
    let s = size / 34.0;
    let o = rect.min;
    let line = Stroke::new(3.2 * s, ON_ACCENT);
    p.line_segment([o + Vec2::new(9.0, 11.0) * s, o + Vec2::new(15.5, 17.0) * s], line);
    p.line_segment([o + Vec2::new(15.5, 17.0) * s, o + Vec2::new(9.0, 23.0) * s], line);
    p.line_segment([o + Vec2::new(18.0, 23.5) * s, o + Vec2::new(26.0, 23.5) * s], line);
}

/// The same mark as a window icon.
fn icon() -> egui::IconData {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    let r = 18.0f32;
    let inside_round = |x: f32, y: f32| {
        let cx = x.clamp(r, n as f32 - r);
        let cy = y.clamp(r, n as f32 - r);
        (x - cx).powi(2) + (y - cy).powi(2) <= r * r
    };
    let seg = |px: f32, py: f32, a: (f32, f32), b: (f32, f32), w: f32| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (px - a.0 - t * dx).powi(2) + (py - a.1 - t * dy).powi(2) <= w * w
    };
    let k = 64.0 / 34.0;
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let i = (y * n + x) * 4;
            if !inside_round(fx, fy) {
                continue;
            }
            let ink = seg(fx, fy, (9.0 * k, 11.0 * k), (15.5 * k, 17.0 * k), 3.0)
                || seg(fx, fy, (15.5 * k, 17.0 * k), (9.0 * k, 23.0 * k), 3.0)
                || seg(fx, fy, (18.0 * k, 23.5 * k), (26.0 * k, 23.5 * k), 3.0);
            let c = if ink { ON_ACCENT } else { ACCENT };
            rgba[i..i + 4].copy_from_slice(&[c.r(), c.g(), c.b(), 255]);
        }
    }
    egui::IconData { rgba, width: n as u32, height: n as u32 }
}

const CHILD_ENV: &str = "LINUXIFY_GUI_CHILD";
const RENDERER_ENV: &str = "LINUXIFY_RENDERER";
/// CREATE_NO_WINDOW: the process gets a console that has no window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn spawn_self(env: &[(&str, &str)]) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(std::env::args_os().skip(1)).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().is_ok()
}

/// Linuxify is a console program (so the CLI's typed confirmations work).
/// For the window, it restarts itself once with a console that has no
/// window: the console it was started with closes (or the terminal gets its
/// prompt back), and the PowerShell, diskpart and VBoxManage commands it runs
/// share the hidden console instead of flashing windows. Returns true when
/// that copy started and this process should just exit.
pub fn restart_without_console() -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    spawn_self(&[(CHILD_ENV, "1")])
}

/// Opens the window and returns when it's closed.
pub fn run(data_dir: Option<PathBuf>, page: Option<&str>) -> Result<(), String> {
    let page = match page {
        Some(p) => Some(Page::parse(p).ok_or_else(|| format!("unknown page '{p}' (use welcome, scan, choose, try, migrate or install)"))?),
        None => None,
    };
    let glow = std::env::var(RENDERER_ENV).is_ok_and(|v| v == "glow");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Linuxify")
            .with_inner_size([1160.0, 780.0])
            .with_min_inner_size([900.0, 600.0])
            .with_icon(icon()),
        renderer: if glow { eframe::Renderer::Glow } else { eframe::Renderer::Wgpu },
        centered: true,
        ..Default::default()
    };
    let result = eframe::run_native("Linuxify", options, Box::new(move |cc| Ok(Box::new(App::new(cc, data_dir, page)))));
    match result {
        Ok(()) => Ok(()),
        // No usable Direct3D/Vulkan adapter: try once more with OpenGL in a fresh
        // process (a window system can only be started once per process).
        Err(_) if !glow && spawn_self(&[(CHILD_ENV, "1"), (RENDERER_ENV, "glow")]) => Ok(()),
        Err(e) => Err(format!("could not open the window ({e}). The command line still works: run `linuxify --help` in a terminal.")),
    }
}

/// Restarts Linuxify as administrator on the given page (Windows asks first).
/// Returns false if the user said no or it couldn't start.
pub fn restart_as_admin(page: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = exe.display().to_string().replace('\'', "''");
    let ps = format!("Start-Process -FilePath '{exe}' -ArgumentList 'gui','--page','{page}' -Verb RunAs");
    std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &ps]).status().is_ok_and(|s| s.success())
}

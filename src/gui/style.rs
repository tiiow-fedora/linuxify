//! Look and feel: colours, fonts and the small widgets every page uses.

use super::job::Progress;
use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, RichText, Stroke, TextStyle, Vec2};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(14, 17, 22);
pub const SIDEBAR: Color32 = Color32::from_rgb(19, 23, 30);
pub const CARD: Color32 = Color32::from_rgb(24, 29, 37);
pub const CARD_HI: Color32 = Color32::from_rgb(32, 38, 49);
pub const BORDER: Color32 = Color32::from_rgb(40, 47, 60);
pub const TEXT: Color32 = Color32::from_rgb(230, 233, 239);
pub const MUTED: Color32 = Color32::from_rgb(154, 163, 178);
pub const FAINT: Color32 = Color32::from_rgb(107, 115, 133);
pub const ACCENT: Color32 = Color32::from_rgb(61, 214, 140);
pub const ON_ACCENT: Color32 = Color32::from_rgb(8, 30, 19);
pub const BLUE: Color32 = Color32::from_rgb(98, 160, 255);
pub const AMBER: Color32 = Color32::from_rgb(245, 181, 68);
pub const RED: Color32 = Color32::from_rgb(242, 109, 109);

/// Segoe UI (every Windows 10/11 has it) when it can be read, else egui's
/// built-in font. egui's own fonts stay as fallbacks for symbols and emoji.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let dir = std::env::var("WINDIR").map(|w| std::path::PathBuf::from(w).join("Fonts")).unwrap_or_default();
    let load = |name: &str| std::fs::read(dir.join(name)).ok().map(|b| Arc::new(egui::FontData::from_owned(b)));
    let regular = load("segoeui.ttf");
    let bold = load("seguisb.ttf").or_else(|| load("segoeuib.ttf"));
    // Segoe UI has no ✔ ⚠ ⌂ 🔒; Segoe UI Symbol does.
    if let Some(sym) = load("seguisym.ttf") {
        fonts.font_data.insert("segoe-symbol".into(), sym);
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe-symbol".into());
    }
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(r) = regular {
        fonts.font_data.insert("segoe".into(), r);
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    let mut bold_chain = fallbacks.clone();
    if let Some(b) = bold {
        fonts.font_data.insert("segoe-bold".into(), b);
        bold_chain.insert(0, "segoe-bold".into());
    } else if fonts.font_data.contains_key("segoe") {
        bold_chain.insert(0, "segoe".into());
    }
    fonts.families.insert(FontFamily::Name("bold".into()), bold_chain);
    ctx.set_fonts(fonts);
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(12.5)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Heading, bold(26.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 7.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.scroll.bar_width = 8.0;
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(14);
        v.extreme_bg_color = Color32::from_rgb(11, 13, 18);
        v.faint_bg_color = CARD_HI;
        v.code_bg_color = Color32::from_rgb(11, 13, 18);
        v.hyperlink_color = BLUE;
        v.warn_fg_color = AMBER;
        v.error_fg_color = RED;
        v.override_text_color = None;
        v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        let r = CornerRadius::same(8);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = CARD;
        w.noninteractive.weak_bg_fill = CARD;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = r;
        w.inactive.bg_fill = CARD_HI;
        w.inactive.weak_bg_fill = CARD_HI;
        w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = r;
        w.hovered.bg_fill = Color32::from_rgb(41, 49, 63);
        w.hovered.weak_bg_fill = Color32::from_rgb(41, 49, 63);
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(70, 82, 102));
        w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
        w.hovered.corner_radius = r;
        w.active.bg_fill = Color32::from_rgb(50, 60, 77);
        w.active.weak_bg_fill = Color32::from_rgb(50, 60, 77);
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.fg_stroke = Stroke::new(1.5, TEXT);
        w.active.corner_radius = r;
        w.open.corner_radius = r;
    });
}

/// A rounded panel for one group of content.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A card tinted with `color` (blockers red, heads-ups amber, info blue...).
pub fn tinted<R>(ui: &mut egui::Ui, color: Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(blend(CARD, color, 0.10))
        .stroke(Stroke::new(1.0, blend(BORDER, color, 0.55)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

fn filled_button(ui: &mut egui::Ui, text: &str, fill: Color32, fg: Color32, enabled: bool) -> egui::Response {
    let label = RichText::new(text).font(bold(15.0)).color(if enabled { fg } else { FAINT });
    let b = egui::Button::new(label).fill(if enabled { fill } else { CARD_HI }).corner_radius(CornerRadius::same(9)).min_size(Vec2::new(0.0, 38.0));
    ui.add_enabled(enabled, b)
}

/// The main action of a page.
pub fn primary(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    filled_button(ui, text, ACCENT, ON_ACCENT, enabled)
}

/// An action that changes the disk.
pub fn danger(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    filled_button(ui, text, RED, Color32::from_rgb(40, 8, 8), enabled)
}

pub fn secondary(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let b = egui::Button::new(RichText::new(text).color(if enabled { TEXT } else { FAINT }))
        .fill(CARD_HI)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(9))
        .min_size(Vec2::new(0.0, 38.0));
    ui.add_enabled(enabled, b)
}

/// A small rounded label, e.g. "Read-only" or "Changes your disk".
pub fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(blend(CARD, color, 0.18))
        .stroke(Stroke::new(1.0, blend(BORDER, color, 0.6)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(9, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.5).color(color));
        });
}

/// Page title with its step number and a risk pill.
pub fn header(ui: &mut egui::Ui, kicker: &str, title: &str, subtitle: &str, risk: Option<(&str, Color32)>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(kicker).size(13.0).color(ACCENT).font(bold(13.0)));
        if let Some((text, color)) = risk {
            pill(ui, text, color);
        }
    });
    ui.label(RichText::new(title).font(bold(28.0)).color(TEXT));
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).size(15.5).color(MUTED));
    }
    ui.add_space(12.0);
}

pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).font(bold(17.0)).color(TEXT));
    ui.add_space(2.0);
}

/// One line with a coloured symbol in front.
pub fn bullet(ui: &mut egui::Ui, symbol: &str, color: Color32, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(symbol).color(color).strong());
        ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap());
    });
}

pub fn muted(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(MUTED)).wrap());
}

/// A big number with a caption, for the scan summary.
pub fn stat(ui: &mut egui::Ui, value: &str, label: &str, color: Color32) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.label(RichText::new(value).font(bold(26.0)).color(color));
                ui.label(RichText::new(label).size(13.0).color(MUTED));
            });
        });
}

pub fn error_box(ui: &mut egui::Ui, title: &str, msg: &str) {
    tinted(ui, RED, |ui| {
        ui.label(RichText::new(title).font(bold(15.5)).color(RED));
        ui.add(egui::Label::new(RichText::new(msg).color(TEXT)).wrap());
    });
}

/// Spinner, stage, bar and the last few log lines of a running job.
pub fn progress(ui: &mut egui::Ui, p: &Progress, elapsed: std::time::Duration) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(18.0).color(ACCENT));
            ui.label(RichText::new(if p.stage.is_empty() { "Starting…" } else { &p.stage }).font(bold(15.5)).color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let s = elapsed.as_secs();
                ui.label(RichText::new(format!("{}:{:02}", s / 60, s % 60)).color(FAINT).monospace());
            });
        });
        ui.add_space(6.0);
        match p.fraction {
            Some(f) => {
                ui.add(egui::ProgressBar::new(f).show_percentage().fill(ACCENT).desired_height(10.0).corner_radius(CornerRadius::same(5)));
            }
            None => busy_bar(ui),
        }
        if !p.detail.is_empty() {
            ui.label(RichText::new(&p.detail).color(MUTED).monospace());
        }
        let earlier: Vec<&String> = p.log.iter().rev().skip(1).take(4).collect();
        if !earlier.is_empty() {
            ui.add_space(4.0);
            for l in earlier.into_iter().rev() {
                ui.label(RichText::new(format!("✓ {l}")).size(13.0).color(FAINT));
            }
        }
    });
}

/// A bar with a sliding stripe, for steps that can't say how far along they are.
fn busy_bar(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 10.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(5), CARD_HI);
    let t = (ui.input(|i| i.time) * 0.6).fract() as f32;
    let w = rect.width() * 0.25;
    let x = rect.left() - w + t * (rect.width() + w);
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(w, rect.height())), CornerRadius::same(5), ACCENT);
    ui.ctx().request_repaint();
}

/// Monospace text in a dark box (commands, scripts, the phrase to type).
pub fn code(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(11, 13, 18))
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).monospace().color(Color32::from_rgb(200, 210, 225))).wrap());
        });
}

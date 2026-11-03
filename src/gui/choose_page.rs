//! Stage 2: the quiz, one question at a time, then the ranked picks.

use super::style::*;
use super::{next_button, open_path, Page, Shared};
use crate::chooser::{self, Answers, Recommendation, QUESTIONS};
use crate::{appdirs, flow};
use crate::webquiz::{QuizResult, QuizSource};
use eframe::egui::{self, CornerRadius, RichText, Sense, Stroke, Vec2};
use std::sync::mpsc;

pub struct ChoosePage {
    answers: Answers,
    /// Index of the question on screen; None shows the results.
    question: Option<usize>,
    result: Option<Recommendation>,
    error: Option<String>,
    /// The website quiz, when one is set up and this build can show it.
    web_source: Option<QuizSource>,
    /// A website quiz window that is open right now (its result arrives here).
    web_job: Option<mpsc::Receiver<Result<Option<QuizResult>, String>>>,
    /// A distro the website quiz picked directly (no answers to rank).
    web_pick: Option<String>,
}

impl ChoosePage {
    pub fn new(sh: &Shared) -> ChoosePage {
        // Show the last result again if there is one.
        let saved = flow::saved_answers().filter(|a| chooser::validate_answers(a).is_ok());
        let result = match (&saved, &sh.distros) {
            (Some(a), Ok(list)) => Some(flow::rank_with_scan(list, a, sh.scan.as_ref())),
            _ => None,
        };
        let web_source = if flow::web_quiz_supported() { flow::quiz_source(None, sh.data_dir.as_deref()).ok().flatten() } else { None };
        ChoosePage { question: if result.is_some() { None } else { Some(0) }, answers: saved.unwrap_or_default(), result, error: None, web_source, web_job: None, web_pick: None }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context, sh: &mut Shared) -> Option<Page> {
        header(ui, "STEP 2", "Choose a distro", "A few plain questions. Your scan is used too (graphics card, memory, Secure Boot, games).", Some(("Read-only", ACCENT)));
        if let Err(e) = &sh.distros {
            error_box(ui, "The distro list couldn't be loaded", e);
            return None;
        }
        if let Some(e) = &self.error {
            error_box(ui, "The quiz result couldn't be used", e);
            ui.add_space(8.0);
        }
        self.poll_web(ui.ctx(), sh);
        if let Some(id) = &self.web_pick {
            let name = sh.distros.as_ref().ok().and_then(|l| l.distros.iter().find(|d| &d.id == id)).map_or(id.clone(), |d| d.name.clone());
            tinted(ui, ACCENT, |ui| muted(ui, &format!("The website quiz picked {name}. The next steps will use it. Answer the questions here too if you also want Linuxify's own ranking.")));
            ui.add_space(8.0);
        }
        match self.question {
            Some(i) => {
                self.quiz(ui, i, sh);
                None
            }
            None => self.results(ui, sh),
        }
    }

    fn quiz(&mut self, ui: &mut egui::Ui, i: usize, sh: &mut Shared) {
        let q = &QUESTIONS[i];
        if sh.scan.is_none() && i == 0 {
            tinted(ui, BLUE, |ui| muted(ui, "No scan yet, so the result uses your answers only. Scanning first (step 1) gives a better match."));
            ui.add_space(8.0);
        }
        if i == 0 {
            self.web_box(ui, sh);
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Question {} of {}", i + 1, QUESTIONS.len())).color(MUTED));
        });
        ui.add(egui::ProgressBar::new(i as f32 / QUESTIONS.len() as f32).fill(ACCENT).desired_height(6.0).corner_radius(CornerRadius::same(3)));
        ui.add_space(14.0);
        ui.label(RichText::new(q.text).font(bold(22.0)).color(TEXT));
        ui.add_space(10.0);
        let current = self.answers.get(q.id).cloned();
        let mut picked = None;
        for o in q.options {
            let selected = current.as_deref() == Some(o.key);
            if option_card(ui, o.label, selected) {
                picked = Some(o.key);
            }
            ui.add_space(6.0);
        }
        if let Some(k) = picked {
            self.answers.insert(q.id.to_string(), k.to_string());
            self.advance(i, sh);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if secondary(ui, "←  Back", i > 0).clicked() {
                self.question = Some(i - 1);
            }
            if current.is_some() && primary(ui, if i + 1 == QUESTIONS.len() { "See my picks" } else { "Next  →" }, true).clicked() {
                self.advance(i, sh);
            }
            if self.result.is_some() && secondary(ui, "Cancel", true).clicked() {
                self.question = None;
            }
        });
    }

    fn advance(&mut self, i: usize, sh: &mut Shared) {
        if i + 1 < QUESTIONS.len() {
            self.question = Some(i + 1);
            return;
        }
        self.finish(sh);
    }

    /// Ranks the current answers, saves the result and shows it.
    fn finish(&mut self, sh: &mut Shared) {
        let Ok(list) = &sh.distros else { return };
        if let Err(e) = chooser::validate_answers(&self.answers) {
            self.error = Some(e);
            return;
        }
        let rec = flow::rank_with_scan(list, &self.answers, sh.scan.as_ref());
        self.error = flow::save_choice(&self.answers, &rec, sh.scan.is_some()).err();
        self.result = Some(rec);
        self.question = None;
        sh.refresh();
    }

    fn web_box(&mut self, ui: &mut egui::Ui, sh: &Shared) {
        let Some(src) = self.web_source.clone() else { return };
        tinted(ui, BLUE, |ui| {
            if self.web_job.is_some() {
                muted(ui, "The website quiz is open in its own window. Finish it there and the result comes back here.");
            } else {
                muted(ui, "Prefer the website version of the quiz? It opens in its own window.");
                if secondary(ui, "Take the quiz on the website", sh.distros.is_ok()).clicked() {
                    if let Ok(list) = sh.distros.clone() {
                        let (tx, rx) = mpsc::channel();
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(flow::run_web_quiz(&src, &list));
                            ctx.request_repaint();
                        });
                        self.web_job = Some(rx);
                        self.error = None;
                    }
                }
            }
        });
        ui.add_space(8.0);
    }

    fn poll_web(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        let Some(rx) = &self.web_job else { return };
        match rx.try_recv() {
            Ok(res) => {
                self.web_job = None;
                match res {
                    Ok(Some(QuizResult::Answers(a))) => {
                        self.answers = a;
                        self.web_pick = None;
                        self.finish(sh);
                    }
                    Ok(Some(QuizResult::Distro(id))) => match flow::save_web_pick(&id) {
                        Ok(_) => {
                            self.web_pick = Some(id);
                            sh.refresh();
                        }
                        Err(e) => self.error = Some(e),
                    },
                    Ok(None) => {}
                    Err(e) => self.error = Some(e),
                }
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(400)),
            Err(mpsc::TryRecvError::Disconnected) => self.web_job = None,
        }
    }

    fn results(&mut self, ui: &mut egui::Ui, sh: &Shared) -> Option<Page> {
        let Some(rec) = &self.result else {
            self.question = Some(0);
            return None;
        };
        for n in &rec.scan_notes {
            bullet(ui, "ⓘ", BLUE, n);
        }
        if !rec.scan_notes.is_empty() {
            ui.add_space(6.0);
        }
        section(ui, "Your top 3");
        for (i, r) in rec.ranking.iter().take(3).enumerate() {
            let accent = if i == 0 { ACCENT } else { BORDER };
            egui::Frame::new()
                .fill(if i == 0 { blend(CARD, ACCENT, 0.06) } else { CARD })
                .stroke(Stroke::new(if i == 0 { 1.5 } else { 1.0 }, accent))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(egui::Margin::same(18))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{}", i + 1)).font(bold(30.0)).color(if i == 0 { ACCENT } else { MUTED }));
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&r.name).font(bold(20.0)).color(TEXT));
                                if i == 0 {
                                    pill(ui, "Best match", ACCENT);
                                }
                            });
                            ui.label(RichText::new(format!("Desktop: {}", r.desktop)).color(MUTED));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("{:.0}%", r.score)).font(bold(22.0)).color(TEXT));
                        });
                    });
                    ui.add(egui::ProgressBar::new((r.score / 100.0) as f32).fill(if i == 0 { ACCENT } else { BLUE }).desired_height(6.0).corner_radius(CornerRadius::same(3)));
                    ui.add_space(4.0);
                    for x in &r.reasons {
                        bullet(ui, "✔", ACCENT, x);
                    }
                    for x in &r.cautions {
                        bullet(ui, "⚠", AMBER, x);
                    }
                    ui.hyperlink_to(RichText::new(&r.homepage).size(13.0), &r.homepage);
                });
            ui.add_space(10.0);
        }
        if rec.ranking.len() > 3 {
            egui::CollapsingHeader::new(RichText::new(format!("The other {} distros", rec.ranking.len() - 3)).color(MUTED)).show(ui, |ui| {
                egui::Grid::new("others").num_columns(3).spacing([18.0, 6.0]).striped(true).show(ui, |ui| {
                    for r in rec.ranking.iter().skip(3) {
                        ui.label(RichText::new(&r.name).color(TEXT));
                        ui.label(RichText::new(format!("{:.0}%", r.score)).color(MUTED));
                        ui.label(RichText::new(&r.desktop).color(FAINT));
                        ui.end_row();
                    }
                });
            });
        }
        if !rec.excluded.is_empty() {
            ui.add_space(4.0);
            egui::CollapsingHeader::new(RichText::new(format!("Left out for this PC ({})", rec.excluded.len())).color(MUTED)).show(ui, |ui| {
                for e in &rec.excluded {
                    bullet(ui, "–", FAINT, &format!("{}: {}", e.name, e.why));
                }
            });
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if secondary(ui, "Retake the quiz", true).clicked() {
                self.question = Some(0);
            }
            if secondary(ui, "Open the result page", true).clicked() {
                if let Ok(h) = appdirs::home() {
                    open_path(&h.join("choice.html"));
                }
            }
            ui.hyperlink_to(RichText::new("Second opinion: distrochooser.de").size(13.5), "https://distrochooser.de/");
        });
        let _ = sh;
        next_button(ui, "Next: try it in a virtual machine  →", Page::Try)
    }
}

/// A big clickable answer.
fn option_card(ui: &mut egui::Ui, text: &str, selected: bool) -> bool {
    let width = ui.available_width().min(720.0);
    let galley = ui.painter().layout(text.to_string(), egui::FontId::proportional(16.0), TEXT, width - 70.0);
    let h = galley.size().y + 28.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, h), Sense::click());
    let hovered = resp.hovered();
    let fill = if selected { blend(CARD, ACCENT, 0.14) } else if hovered { CARD_HI } else { CARD };
    let stroke = if selected { Stroke::new(1.5, ACCENT) } else { Stroke::new(1.0, if hovered { MUTED } else { BORDER }) };
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(12), fill, stroke, egui::StrokeKind::Inside);
    let c = egui::Pos2::new(rect.left() + 24.0, rect.center().y);
    p.circle_stroke(c, 9.0, Stroke::new(1.5, if selected { ACCENT } else { MUTED }));
    if selected {
        p.circle_filled(c, 5.0, ACCENT);
    }
    p.galley(egui::Pos2::new(rect.left() + 46.0, rect.center().y - galley.size().y / 2.0), galley, TEXT);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

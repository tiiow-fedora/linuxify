//! Long steps (scan, download, copy, install) run on a worker thread so the
//! window stays responsive. The worker reports through a [`Reporter`]; the
//! page polls its [`Job`] every frame.

use eframe::egui;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// What a running job last said about itself.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    /// What it is doing now, e.g. "Downloading from mirrors.edge.kernel.org".
    pub stage: String,
    /// Fraction done (0..=1), when the step can tell.
    pub fraction: Option<f32>,
    /// A short detail line, e.g. "1.20 GB / 2.80 GB · 11.3 MB/s".
    pub detail: String,
    /// Every stage and note so far, oldest first.
    pub log: Vec<String>,
}

#[derive(Clone)]
pub struct Reporter {
    shared: Arc<Mutex<Progress>>,
    ctx: egui::Context,
}

impl Reporter {
    fn update(&self, f: impl FnOnce(&mut Progress)) {
        if let Ok(mut p) = self.shared.lock() {
            f(&mut p);
        }
        self.ctx.request_repaint();
    }

    /// A new step: replaces the stage, clears the bar and adds it to the log.
    pub fn stage(&self, s: impl Into<String>) {
        let s = s.into();
        self.update(|p| {
            p.log.push(s.clone());
            p.stage = s;
            p.fraction = None;
            p.detail.clear();
        });
    }

    /// A line for the log that doesn't change the stage.
    pub fn note(&self, s: impl Into<String>) {
        let s = s.into();
        self.update(|p| p.log.push(s));
    }

    pub fn fraction(&self, fraction: Option<f32>, detail: impl Into<String>) {
        let detail = detail.into();
        self.update(|p| {
            p.fraction = fraction.map(|f| f.clamp(0.0, 1.0));
            p.detail = detail;
        });
    }

    /// Progress over a byte count, e.g. "1.20 GB / 2.80 GB".
    pub fn bytes(&self, done: u64, total: u64, extra: &str) {
        let f = if total > 0 { Some(done as f32 / total as f32) } else { None };
        let human = crate::download::human;
        let total_text = if total > 0 { format!(" / {}", human(total)) } else { String::new() };
        self.fraction(f, format!("{}{total_text}{extra}", human(done)));
    }

    /// Forwards download/checksum events to this job's progress.
    pub fn download_events(&self) -> impl FnMut(crate::download::Event) + '_ {
        use crate::download::Event;
        move |e| match e {
            Event::Note(m) => self.note(m),
            Event::DownloadStart { resume_at, total, host } => {
                let what = if resume_at > 0 { "Resuming the download" } else { "Downloading" };
                self.stage(format!("{what} from {host} ({})", crate::download::human(total)));
            }
            Event::Downloading { done, total, bytes_per_sec } => self.bytes(done, total, &format!("  ·  {:.1} MB/s", bytes_per_sec / 1e6)),
            Event::DownloadDone { bytes } => self.note(format!("Downloaded {}.", crate::download::human(bytes))),
            Event::HashStart { .. } => self.stage("Checking the SHA-256 against the official checksum"),
            Event::Hashing { done, total } => self.bytes(done, total, ""),
            Event::HashDone => self.note("Checksum computed."),
        }
    }
}

pub struct Job<T> {
    shared: Arc<Mutex<Progress>>,
    rx: mpsc::Receiver<Result<T, String>>,
    pub started: Instant,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(ctx: &egui::Context, f: impl FnOnce(&Reporter) -> Result<T, String> + Send + 'static) -> Job<T> {
        let shared = Arc::new(Mutex::new(Progress::default()));
        let (tx, rx) = mpsc::channel();
        let reporter = Reporter { shared: shared.clone(), ctx: ctx.clone() };
        std::thread::spawn(move || {
            // A bug in a step must show as an error, not leave the page spinning forever.
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&reporter)))
                .unwrap_or_else(|_| Err("Linuxify hit an internal error during this step (nothing more was done). Please report it.".into()));
            let _ = tx.send(r);
            reporter.ctx.request_repaint();
        });
        Job { shared, rx, started: Instant::now() }
    }

    /// The result, once (Some exactly one time).
    pub fn poll(&self) -> Option<Result<T, String>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("the step stopped unexpectedly".into())),
        }
    }

    pub fn progress(&self) -> Progress {
        self.shared.lock().map(|p| p.clone()).unwrap_or_default()
    }
}

/// A job slot on a page: idle, running, or finished with a result.
#[derive(Default)]
pub enum Task<T> {
    #[default]
    Idle,
    Running(Job<T>),
    Done(Result<T, String>),
}

impl<T: Send + 'static> Task<T> {
    pub fn start(&mut self, ctx: &egui::Context, f: impl FnOnce(&Reporter) -> Result<T, String> + Send + 'static) {
        *self = Task::Running(Job::spawn(ctx, f));
    }

    /// Moves a finished job into `Done`; true on the frame it finished. Call once per frame.
    pub fn tick(&mut self) -> bool {
        if let Task::Running(j) = self {
            if let Some(r) = j.poll() {
                *self = Task::Done(r);
                return true;
            }
        }
        false
    }

    pub fn running(&self) -> bool {
        matches!(self, Task::Running(_))
    }

    pub fn ok(&self) -> Option<&T> {
        match self {
            Task::Done(Ok(t)) => Some(t),
            _ => None,
        }
    }

    pub fn err(&self) -> Option<&str> {
        match self {
            Task::Done(Err(e)) => Some(e),
            _ => None,
        }
    }
}

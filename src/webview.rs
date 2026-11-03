//! The window that shows the website quiz (feature `webview`).
//!
//! It runs in its own process (`linuxify quiz-window`) so its event loop never
//! fights the main window's. It loads the quiz address, applies the navigation
//! policy from `webquiz`, and writes the page's result (untrusted text) to a
//! file. The caller validates that text again with `webquiz::parse_result`.
//!
//! This file is a thin shell; the rules live in `webquiz` and are unit-tested.
//! It has been compiled on Linux (WebKitGTK) only. The Windows (WebView2) build and
//! behaviour are untested; CI builds it on Windows.

use crate::webquiz::{self, Nav, QuizSource};
use std::path::Path;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::{NewWindowResponse, PermissionResponse, WebViewBuilder};

enum Ev {
    Result(String),
    Open(String),
}

/// Opens the window and waits. Only returns (with an error) when the window could not be
/// opened; otherwise the process exits, see below.
pub fn run(source: QuizSource, out: &Path) -> Result<std::convert::Infallible, String> {
    let start = match &source {
        QuizSource::Url(u) => u.clone(),
        QuizSource::File(f) => {
            if !f.is_file() {
                return Err(format!("the quiz file {} does not exist", f.display()));
            }
            let name = f.file_name().and_then(|n| n.to_str()).ok_or("the quiz file has an unusable name")?;
            // Windows (WebView2) shows custom schemes as http://<scheme>.localhost/.
            if cfg!(windows) { format!("http://{}/{name}", webquiz::ASSET_HOST) } else { format!("{}://localhost/{name}", webquiz::ASSET_SCHEME) }
        }
    };
    let asset_root = match &source {
        QuizSource::File(f) => f.parent().map(|p| p.to_path_buf()),
        QuizSource::Url(_) => None,
    };
    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("Linuxify: which Linux is right for you?")
        .with_inner_size(LogicalSize::new(1000.0, 760.0))
        .build(&event_loop)
        .map_err(|e| format!("could not open the quiz window: {e}"))?;

    let (p_nav, p_ipc, p_new) = (proxy.clone(), proxy.clone(), proxy);
    let (s_nav, s_new) = (source.clone(), source.clone());
    let builder = WebViewBuilder::new()
        .with_url(&start)
        .with_devtools(false)
        .with_navigation_handler(move |url| {
            if std::env::var_os("LINUXIFY_DEBUG").is_some() {
                eprintln!("quiz window navigation: {url}");
            }
            match webquiz::nav_decision(&s_nav, &url) {
            Nav::Allow => true,
            Nav::Result(r) => {
                let _ = p_nav.send_event(Ev::Result(r));
                false
            }
            Nav::OpenExternally => {
                let _ = p_nav.send_event(Ev::Open(url));
                false
            }
            Nav::Block => false,
            }
        })
        .with_new_window_req_handler(move |url, _| {
            // Links that want a new window: the browser for https, otherwise nothing.
            match webquiz::nav_decision(&s_new, &url) {
                Nav::Result(r) => {
                    let _ = p_new.send_event(Ev::Result(r));
                }
                Nav::Allow | Nav::OpenExternally => {
                    let _ = p_new.send_event(Ev::Open(url));
                }
                Nav::Block => {}
            }
            NewWindowResponse::Deny
        })
        .with_custom_protocol(webquiz::ASSET_SCHEME.to_string(), move |_id, req| serve_asset(asset_root.as_deref(), req.uri().path()))
        .with_permission_handler(|_| PermissionResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_ipc_handler(move |req| {
            if std::env::var_os("LINUXIFY_DEBUG").is_some() {
                eprintln!("quiz window message: {} bytes", req.body().len());
            }
            let _ = p_ipc.send_event(Ev::Result(req.body().clone()));
        });

    #[cfg(target_os = "windows")]
    let _webview = builder.build(&window).map_err(|e| format!("could not start the web view (is the Microsoft Edge WebView2 runtime installed?): {e}"))?;
    #[cfg(not(target_os = "windows"))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().ok_or("no window area for the web view")?;
        builder.build_gtk(vbox).map_err(|e| format!("could not start the web view: {e}"))?
    };

    let out = out.to_path_buf();
    // `run` never returns; the process exits with 0 (result saved), 2 (window closed
    // without a result) or 1 (the result could not be saved). This only ever runs in
    // the `quiz-window` child process.
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let _keep_alive = &_webview;
        match event {
            Event::UserEvent(Ev::Result(text)) => {
                // The size is checked here too: never write more than the limit to disk.
                if text.len() <= webquiz::MAX_RESULT_BYTES {
                    match std::fs::write(&out, text) {
                        Ok(()) => std::process::exit(0),
                        Err(e) => {
                            eprintln!("could not save the quiz result: {e}");
                            std::process::exit(1)
                        }
                    }
                }
            }
            Event::UserEvent(Ev::Open(url)) => {
                if webquiz::safe_to_open_externally(&url) {
                    open_in_browser(&url);
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => std::process::exit(2),
            _ => {}
        }
    })
}

/// Serves one file of a local quiz folder (read-only, allow-listed file types, no escaping).
fn serve_asset(root: Option<&Path>, path: &str) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    let reply = |code: u16, mime: &str, body: Vec<u8>| {
        wry::http::Response::builder().status(code).header("Content-Type", mime).body(std::borrow::Cow::Owned(body)).unwrap_or_default()
    };
    let Some(file) = root.and_then(|r| webquiz::resolve_asset(r, path)) else { return reply(404, "text/plain", b"not found".to_vec()) };
    let Some(mime) = webquiz::asset_mime(&file) else { return reply(404, "text/plain", b"not found".to_vec()) };
    match std::fs::read(&file) {
        Ok(bytes) if bytes.len() <= 32 * 1024 * 1024 => reply(200, mime, bytes),
        _ => reply(404, "text/plain", b"not found".to_vec()),
    }
}

#[cfg(target_os = "windows")]
fn open_in_browser(url: &str) {
    // `explorer <url>` opens the default browser without going through a shell.
    let _ = std::process::Command::new("explorer").arg(url).spawn();
}

#[cfg(not(target_os = "windows"))]
fn open_in_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

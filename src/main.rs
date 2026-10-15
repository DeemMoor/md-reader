#![windows_subsystem = "windows"]

mod render;

use std::borrow::Cow;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};
use std::{fs, thread};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;
use wry::http::{Request, Response, header::CONTENT_TYPE};

use render::{SYNTAX_CSS, escape_html, markdown_to_html};

const APP_NAME: &str = "MD Reader";
const PROTOCOL: &str = "mdr";
const STYLE: &str = include_str!("style.css");
const SCRIPT: &str = include_str!("app.js");
const MARKDOWN_EXTS: &[&str] = &["md", "markdown", "mdown", "mkd", "mkdn", "mdx"];

// WebView2 serves custom protocols through http://<name>.localhost.
#[cfg(windows)]
const BASE_URL: &str = "http://mdr.localhost/";
#[cfg(not(windows))]
const BASE_URL: &str = "mdr://localhost/";

/// Per-process nonce: only our own script may run, never scripts embedded in a document.
static NONCE: LazyLock<String> =
    LazyLock::new(|| format!("{:016x}", RandomState::new().build_hasher().finish()));

type Current = Arc<Mutex<Option<PathBuf>>>;

#[derive(Debug)]
enum UserEvent {
    OpenDialog,
    Open(PathBuf),
    Refresh,
    Title(String),
}

fn main() -> wry::Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title(APP_NAME)
        .with_inner_size(LogicalSize::new(1000.0, 820.0))
        .with_min_inner_size(LogicalSize::new(400.0, 300.0))
        .build(&event_loop)
        .expect("failed to create window");

    let current: Current = Arc::new(Mutex::new(None));
    let start_url = match std::env::args_os().nth(1) {
        Some(arg) => doc_url(&std::path::absolute(PathBuf::from(arg)).unwrap_or_default()),
        None => BASE_URL.to_string(),
    };

    let webview = WebViewBuilder::new()
        .with_custom_protocol(PROTOCOL.into(), {
            let current = current.clone();
            move |_id, request| serve(&request, &current)
        })
        .with_navigation_handler(handle_navigation)
        .with_ipc_handler({
            let proxy = proxy.clone();
            move |req: Request<String>| {
                if req.body() == "open" {
                    let _ = proxy.send_event(UserEvent::OpenDialog);
                }
            }
        })
        .with_drag_drop_handler({
            let proxy = proxy.clone();
            move |event| {
                if let wry::DragDropEvent::Drop { paths, .. } = event
                    && let Some(path) = paths.into_iter().find(|p| p.is_file())
                {
                    let _ = proxy.send_event(UserEvent::Open(path));
                }
                true
            }
        })
        .with_document_title_changed_handler({
            let proxy = proxy.clone();
            move |title| {
                let _ = proxy.send_event(UserEvent::Title(title));
            }
        })
        .with_url(start_url)
        .build(&window)?;

    spawn_watcher(current.clone(), proxy);
    let _ = webview.focus();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::Exit;
            }
            // Keep keyboard scrolling working without clicking into the page first.
            Event::WindowEvent { event: WindowEvent::Focused(true), .. } => {
                let _ = webview.focus();
            }
            Event::UserEvent(UserEvent::OpenDialog) => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Открыть Markdown")
                    .add_filter("Markdown", MARKDOWN_EXTS)
                    .add_filter("Все файлы", &["*"])
                    .set_parent(&window);
                if let Some(dir) = current.lock().unwrap().as_ref().and_then(|p| p.parent()) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(path) = dialog.pick_file() {
                    let _ = webview.load_url(&doc_url(&path));
                }
            }
            Event::UserEvent(UserEvent::Open(path)) => {
                let _ = webview.load_url(&doc_url(&path));
            }
            Event::UserEvent(UserEvent::Refresh) => {
                let _ = webview.evaluate_script("window.mdrRefresh && window.mdrRefresh()");
            }
            Event::UserEvent(UserEvent::Title(title)) => {
                window.set_title(&if title.is_empty() || title == APP_NAME {
                    APP_NAME.to_string()
                } else {
                    format!("{title} — {APP_NAME}")
                });
            }
            _ => {}
        }
    });
}

/// Local URLs stay in the viewer (markdown) or go to the default app (other files);
/// everything else opens in the system browser / mail client.
fn handle_navigation(url: String) -> bool {
    if url.starts_with(BASE_URL) || url.starts_with("mdr:") {
        if let Some(path) = path_from_url(&url)
            && !is_markdown(&path)
            && path.is_file()
        {
            let _ = open::that_detached(&path);
            return false;
        }
        return true;
    }
    if url.starts_with("about:") {
        return true;
    }
    let _ = open::that_detached(&url);
    false
}

fn serve(request: &Request<Vec<u8>>, current: &Current) -> Response<Cow<'static, [u8]>> {
    let raw = request.uri().path().trim_start_matches('/');
    if raw.is_empty() {
        *current.lock().unwrap() = None;
        return html_response(welcome_page());
    }

    let path = PathBuf::from(percent_decode(raw));
    let body_only = request.uri().query() == Some("body");

    if is_markdown(&path) {
        let title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let content = match fs::read(&path) {
            Ok(bytes) => markdown_to_html(&String::from_utf8_lossy(&bytes)),
            Err(err) => format!(
                r#"<p class="error">Не удалось открыть <code>{}</code>: {}</p>"#,
                escape_html(&path.display().to_string()),
                escape_html(&err.to_string())
            ),
        };
        if body_only {
            return html_response(content);
        }
        *current.lock().unwrap() = Some(path);
        return html_response(page(&title, &format!(r#"<article id="content" class="markdown-body">{content}</article>"#)));
    }

    match fs::read(&path) {
        Ok(bytes) => Response::builder()
            .header(CONTENT_TYPE, mime_for(&path))
            .body(Cow::Owned(bytes))
            .unwrap(),
        Err(_) => Response::builder().status(404).body(Cow::Borrowed(&[][..])).unwrap(),
    }
}

fn page(title: &str, body: &str) -> String {
    let nonce = &*NONCE;
    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'self' data: blob:; img-src * data: blob:; media-src * data: blob:; style-src 'self' 'unsafe-inline'; script-src 'nonce-{nonce}'">
<title>{title}</title>
<style>{STYLE}
{css}</style>
</head>
<body>
{body}
<script nonce="{nonce}">{SCRIPT}</script>
</body>
</html>"#,
        title = escape_html(title),
        css = &*SYNTAX_CSS,
    )
}

fn welcome_page() -> String {
    page(
        APP_NAME,
        r#"<div class="welcome">
<h1>MD Reader</h1>
<p>Перетащи сюда <code>.md</code> файл</p>
<p>или нажми <kbd>Ctrl</kbd> + <kbd>O</kbd></p>
</div>"#,
    )
}

fn html_response(html: String) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header("Cache-Control", "no-store")
        .body(Cow::Owned(html.into_bytes()))
        .unwrap()
}

/// Polls the open document and asks the page to re-render when it changes on disk.
fn spawn_watcher(current: Current, proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let mut last: Option<(PathBuf, Option<SystemTime>)> = None;
        loop {
            thread::sleep(Duration::from_millis(400));
            let Some(path) = current.lock().unwrap().clone() else {
                last = None;
                continue;
            };
            let mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
            if let Some((prev_path, prev_mtime)) = &last
                && *prev_path == path
                && *prev_mtime != mtime
                && proxy.send_event(UserEvent::Refresh).is_err()
            {
                return;
            }
            last = Some((path, mtime));
        }
    });
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MARKDOWN_EXTS.iter().any(|m| m.eq_ignore_ascii_case(e)))
}

fn doc_url(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut url = String::from(BASE_URL);
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/:-_.~".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

fn path_from_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix(BASE_URL)?;
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    (!rest.is_empty()).then(|| PathBuf::from(percent_decode(rest)))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn mime_for(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "css" => "text/css",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

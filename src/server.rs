//! The `mdr://` protocol: the app shell, rendered documents, embedded assets and a tiny
//! JSON API. Everything the webview shows comes from here.

use std::borrow::Cow;
use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use rust_embed::RustEmbed;
use serde::Serialize;
use wry::http::{Request, Response, header::CONTENT_TYPE};

use crate::render::{SYNTAX_CSS, escape_html, markdown_to_html, source_to_html};
use crate::settings::Settings;

pub const PROTOCOL: &str = "mdr";

// WebView2 serves custom protocols through http://<name>.localhost.
#[cfg(windows)]
pub const BASE_URL: &str = "http://mdr.localhost/";
#[cfg(not(windows))]
pub const BASE_URL: &str = "mdr://localhost/";

pub const MARKDOWN_EXTS: &[&str] = &["md", "markdown", "mdown", "mkd", "mkdn", "mdx"];

const PLATFORM: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "macos"
} else {
    "linux"
};

#[derive(RustEmbed)]
#[folder = "assets/"]
#[exclude = "icons/*.ico"]
#[exclude = "icons/*@2x.png"]
struct Assets;

/// Per-process nonce: only our own scripts may run, never scripts embedded in a document.
static NONCE: LazyLock<String> =
    LazyLock::new(|| format!("{:016x}", RandomState::new().build_hasher().finish()));

pub fn handle(request: &Request<Vec<u8>>, settings: &Arc<Mutex<Settings>>) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    // Strip exactly one slash: Unix paths keep their own leading "/" ("//home/x.md").
    let raw = uri.path().strip_prefix('/').unwrap_or(uri.path());
    let query = Query(uri.query().unwrap_or(""));

    if raw.is_empty() {
        let settings = settings.lock().unwrap().clone();
        return html(shell_page(&settings));
    }
    if let Some(asset) = raw.strip_prefix("_app/") {
        return match Assets::get(asset) {
            Some(file) => respond(mime_for(Path::new(asset)), file.data.into_owned()),
            None => not_found(),
        };
    }
    if raw == "_api/list" {
        let dir = PathBuf::from(query.get("dir").unwrap_or_default());
        return json(&list_dir(&dir));
    }

    let path = PathBuf::from(percent_decode(raw));
    if is_markdown(&path) {
        let source = query.get("view").as_deref() == Some("source");
        let content = match fs::read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                if source { source_to_html(&text) } else { markdown_to_html(&text) }
            }
            Err(err) => format!(
                r#"<p class="error">Не удалось открыть <code>{}</code>: {}</p>"#,
                escape_html(&path.display().to_string()),
                escape_html(&err.to_string())
            ),
        };
        if query.has("body") {
            return html(content);
        }
        let settings = settings.lock().unwrap().clone();
        return html(doc_page(&path, &content, &settings));
    }

    match fs::read(&path) {
        Ok(bytes) => respond(mime_for(&path), bytes),
        Err(_) => not_found(),
    }
}

fn csp() -> String {
    format!(
        "default-src 'self' data: blob:; img-src * data: blob:; media-src * data: blob:; \
         style-src 'self' 'unsafe-inline'; font-src 'self' data:; script-src 'nonce-{}'",
        &*NONCE
    )
}

#[derive(Serialize)]
struct Init<'a> {
    settings: &'a Settings,
    platform: &'static str,
    version: &'static str,
    about: About,
}

/// Facts for the "About" window.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct About {
    version: &'static str,
    build_date: &'static str,
    commit: &'static str,
    os: String,
    engine: String,
    exe: String,
    settings_file: String,
    data_dir: String,
}

fn about() -> About {
    let os = match PLATFORM {
        "windows" => "Windows",
        "macos" => "macOS",
        _ => "Linux",
    };
    let engine = if cfg!(windows) {
        "WebView2"
    } else if cfg!(target_os = "macos") {
        "WebKit"
    } else {
        "WebKitGTK"
    };
    let path = |p: Option<PathBuf>| p.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    About {
        version: env!("CARGO_PKG_VERSION"),
        build_date: env!("MDR_BUILD_DATE"),
        commit: env!("MDR_COMMIT"),
        os: format!("{os} {}", std::env::consts::ARCH),
        engine: match wry::webview_version() {
            Ok(v) => format!("{engine} {v}"),
            Err(_) => engine.to_string(),
        },
        exe: path(std::env::current_exe().ok()),
        settings_file: path(crate::settings::settings_path()),
        data_dir: path(crate::settings::webview_data_dir().and_then(|d| d.parent().map(PathBuf::from))),
    }
}

fn shell_page(settings: &Settings) -> String {
    let init = Init { settings, platform: PLATFORM, version: env!("CARGO_PKG_VERSION"), about: about() };
    // Keep "</script>" in file names from closing the inline script.
    let init = serde_json::to_string(&init).unwrap_or_default().replace("</", "<\\/");
    format!(
        r#"<!doctype html>
<html data-theme="{theme}" data-platform="{PLATFORM}">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<title>MD Reader</title>
<link rel="stylesheet" href="/_app/shell.css">
</head>
<body>
<header id="tabbar">
  <button id="sidebar-toggle" class="icon-btn" title="Боковая панель (Ctrl+B)"><svg viewBox="0 0 16 16"><path d="M2 3.5h12M2 8h12M2 12.5h12"/></svg></button>
  <div id="tabs" role="tablist"></div>
  <button id="open-btn" class="icon-btn" title="Открыть файл (Ctrl+O)"><svg viewBox="0 0 16 16"><path d="M8 3v10M3 8h10"/></svg></button>
</header>
<div id="main">
  <aside id="sidebar">
    <nav class="sidebar-tabs">
      <button data-panel="toc">Оглавление</button>
      <button data-panel="files">Файлы</button>
    </nav>
    <div id="toc" class="panel"></div>
    <div id="files" class="panel"></div>
  </aside>
  <div id="resizer"></div>
  <section id="docs">
    <div id="welcome">
      <h1>MD Reader</h1>
      <p>Перетащи сюда <code>.md</code> файл или нажми <kbd>Ctrl</kbd> + <kbd>O</kbd></p>
      <button id="welcome-open" class="primary">Открыть файл…</button>
      <div id="welcome-recent"></div>
    </div>
  </section>
</div>
<div id="about" class="modal" hidden></div>
<script nonce="{nonce}">window.MDR_INIT = {init};</script>
<script nonce="{nonce}" src="/_app/shell.js"></script>
</body>
</html>"#,
        theme = escape_html(&settings.theme),
        csp = csp(),
        nonce = &*NONCE,
    )
}

fn doc_page(path: &Path, content: &str, settings: &Settings) -> String {
    let title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    format!(
        r#"<!doctype html>
<html data-theme="{theme}" data-width="{width}" style="zoom:{zoom}">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<title>{title}</title>
<link rel="stylesheet" href="/_app/doc.css">
<style>{css}</style>
</head>
<body>
<article id="content" class="markdown-body">{content}</article>
<script nonce="{nonce}" src="/_app/doc.js"></script>
</body>
</html>"#,
        theme = escape_html(&settings.theme),
        width = escape_html(&settings.width),
        zoom = settings.zoom,
        csp = csp(),
        title = escape_html(&title),
        css = &*SYNTAX_CSS,
        nonce = &*NONCE,
    )
}

/// Self-contained HTML for "Export to HTML": the rendered article plus all styles inline.
pub fn export_page(title: &str, article: &str) -> String {
    let doc_css = Assets::get("doc.css").map(|f| String::from_utf8_lossy(&f.data).into_owned());
    let katex_css = Assets::get("vendor/katex/katex.min.css")
        .map(|f| String::from_utf8_lossy(&f.data).into_owned());
    format!(
        r#"<!doctype html>
<html data-theme="light" data-width="narrow">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>{doc_css}
{syntax}
{katex_css}
.code-copy, .anchor {{ display: none !important; }}</style>
</head>
<body>
<article class="markdown-body">{article}</article>
</body>
</html>"#,
        title = escape_html(title),
        doc_css = doc_css.unwrap_or_default(),
        syntax = &*SYNTAX_CSS,
        katex_css = katex_css.unwrap_or_default(),
    )
}

#[derive(Serialize)]
struct Listing {
    dir: String,
    parent: Option<String>,
    entries: Vec<Entry>,
}

#[derive(Serialize)]
struct Entry {
    name: String,
    path: String,
    dir: bool,
}

fn list_dir(dir: &Path) -> Listing {
    let mut entries: Vec<Entry> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let path = e.path();
            let is_dir = path.is_dir();
            (is_dir || is_markdown(&path)).then(|| Entry {
                name,
                path: path.to_string_lossy().into_owned(),
                dir: is_dir,
            })
        })
        .collect();
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Listing {
        dir: dir.to_string_lossy().into_owned(),
        parent: dir.parent().map(|p| p.to_string_lossy().into_owned()),
        entries,
    }
}

pub fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MARKDOWN_EXTS.iter().any(|m| m.eq_ignore_ascii_case(e)))
}

#[cfg(test)]
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

pub fn percent_decode(s: &str) -> String {
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

struct Query<'a>(&'a str);

impl Query<'_> {
    fn pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| p.split_once('=').unwrap_or((p, "")))
    }

    fn get(&self, key: &str) -> Option<String> {
        self.pairs().find(|(k, _)| *k == key).map(|(_, v)| percent_decode(v))
    }

    fn has(&self, key: &str) -> bool {
        self.pairs().any(|(k, _)| k == key)
    }
}

fn mime_for(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "woff2" => "font/woff2",
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
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn respond(mime: &str, body: Vec<u8>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .header(CONTENT_TYPE, mime)
        .header("Cache-Control", "no-store")
        .body(Cow::Owned(body))
        .unwrap()
}

fn html(body: String) -> Response<Cow<'static, [u8]>> {
    respond("text/html; charset=utf-8", body.into_bytes())
}

fn json<T: Serialize>(value: &T) -> Response<Cow<'static, [u8]>> {
    respond("application/json", serde_json::to_vec(value).unwrap_or_default())
}

fn not_found() -> Response<Cow<'static, [u8]>> {
    Response::builder().status(404).body(Cow::Borrowed(&[][..])).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_url_round_trips() {
        let path = Path::new("C:/Мои документы/a b#1.md");
        let url = doc_url(path);
        let encoded = url.strip_prefix(BASE_URL).unwrap();
        assert!(!encoded.contains(' ') && !encoded.contains('#'));
        assert_eq!(percent_decode(encoded), "C:/Мои документы/a b#1.md");
    }

    #[test]
    fn query_parsing() {
        let q = Query("view=source&body&dir=C%3A%2Fx");
        assert_eq!(q.get("view").as_deref(), Some("source"));
        assert!(q.has("body"));
        assert_eq!(q.get("dir").as_deref(), Some("C:/x"));
    }
}

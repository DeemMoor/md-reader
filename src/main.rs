#![windows_subsystem = "windows"]

mod clipboard;
mod instance;
mod links;
mod log;
mod menu;
mod platform;
mod render;
mod server;
mod settings;
mod update;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fs, thread};

use muda::MenuEvent;
use serde_json::{Value, json};
use tao::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Fullscreen, Icon, Theme, Window, WindowBuilder};
use wry::http::Request;
use wry::{WebView, WebViewBuilder};

use menu::AppMenu;
use server::{BASE_URL, MARKDOWN_EXTS, PROTOCOL};
use settings::{SessionTab, Settings, WindowState, same_path};

const APP_NAME: &str = "MD Reader";
const ICON_PNG: &[u8] = include_bytes!("../assets/icons/icon-256.png");

#[derive(Debug)]
enum UserEvent {
    Menu(String),
    Ipc(String),
    Open(Vec<PathBuf>),
    Refresh(PathBuf),
    Title(String),
    /// Script for the page, from a background thread.
    Js(String),
    /// A downloaded update installer: start it and quit.
    RunInstaller(PathBuf),
}

fn main() -> wry::Result<()> {
    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let from_elevated = args.iter().any(|a| a == "--from-elevated");
    args.retain(|a| a != "--from-elevated");

    #[cfg(windows)]
    if !from_elevated && platform::relaunch_unelevated(&args) {
        return Ok(());
    }
    #[cfg(not(windows))]
    let _ = from_elevated;

    let paths: Vec<PathBuf> = args
        .iter()
        .map(|a| std::path::absolute(a).unwrap_or_else(|_| PathBuf::from(a)))
        .collect();
    if instance::forward_to_running(&paths) {
        return Ok(());
    }

    log::init();
    let settings = Arc::new(Mutex::new(Settings::load()));
    clipboard::cleanup(&settings.lock().unwrap().session.iter().map(|t| t.path.clone()).collect::<Vec<_>>());
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    instance::listen({
        let proxy = proxy.clone();
        move |paths| {
            let _ = proxy.send_event(UserEvent::Open(paths));
        }
    });
    MenuEvent::set_event_handler(Some({
        let proxy = proxy.clone();
        move |event: MenuEvent| {
            let _ = proxy.send_event(UserEvent::Menu(event.id.0));
        }
    }));

    let initial = settings.lock().unwrap().clone();
    let mut builder = WindowBuilder::new()
        .with_title(APP_NAME)
        .with_inner_size(LogicalSize::new(1100.0, 820.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0))
        .with_window_icon(window_icon())
        .with_theme(window_theme(&initial.theme));
    if let Some(ws) = initial.window {
        let on_screen = event_loop.available_monitors().any(|m| {
            let (p, s) = (m.position(), m.size());
            ws.x + 50 >= p.x && ws.y >= p.y && ws.x + 50 < p.x + s.width as i32 && ws.y + 50 < p.y + s.height as i32
        });
        builder = builder.with_inner_size(PhysicalSize::new(ws.width, ws.height)).with_maximized(ws.maximized);
        if on_screen {
            builder = builder.with_position(PhysicalPosition::new(ws.x, ws.y));
        }
    }
    let window = builder.build(&event_loop).expect("failed to create window");

    let app_menu = AppMenu::new(&initial);
    #[cfg(windows)]
    unsafe {
        use tao::platform::windows::WindowExtWindows;
        let _ = app_menu.menu.init_for_hwnd_with_theme(window.hwnd(), menu_theme(&initial.theme));
    }
    #[cfg(target_os = "macos")]
    app_menu.menu.init_for_nsapp();
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = app_menu.menu.init_for_gtk_window(window.gtk_window(), window.default_vbox());
    }

    // WebView2 keeps its profile next to the exe by default, which is not writable
    // when installed into Program Files.
    let mut web_context = wry::WebContext::new(settings::webview_data_dir());
    let webview = match build_webview(&window, &mut web_context, &settings, &proxy) {
        Ok(webview) => webview,
        Err(err) => {
            log::error(format!("WebView2 failed to start: {err}"));
            rfd::MessageDialog::new()
                .set_title(APP_NAME)
                .set_description(format!(
                    "Не удалось запустить встроенный браузер (WebView2).\n\n{err}\n\n\
                     Установите Microsoft Edge WebView2 Runtime и попробуйте снова."
                ))
                .set_level(rfd::MessageLevel::Error)
                .show();
            return Err(err);
        }
    };
    let watched: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    spawn_watcher(watched.clone(), proxy.clone());

    let mut app = App {
        window,
        webview,
        _web_context: web_context,
        menu: app_menu,
        settings,
        watched,
        pending: paths,
        ready: false,
        proxy: proxy.clone(),
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                app.save_window_state();
                *control_flow = ControlFlow::Exit;
            }
            // Keep keyboard scrolling working without clicking into the page first.
            Event::WindowEvent { event: WindowEvent::Focused(true), .. } => {
                let _ = app.webview.focus();
            }
            Event::UserEvent(UserEvent::Menu(id)) => {
                if app.handle_command(&id) {
                    app.save_window_state();
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::Ipc(body)) => {
                if app.handle_ipc(&body) {
                    app.save_window_state();
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::Open(paths)) => {
                app.open(paths);
                app.bring_to_front();
            }
            Event::UserEvent(UserEvent::Refresh(path)) => {
                app.js(&format!("mdrShell.fileChanged({})", json!(path)));
            }
            Event::UserEvent(UserEvent::Js(script)) => app.js(&script),
            Event::UserEvent(UserEvent::RunInstaller(path)) => {
                // Through the shell, so the installer can ask for elevation if it needs it.
                match open::that_detached(&path) {
                    Ok(()) => {
                        app.save_window_state();
                        *control_flow = ControlFlow::Exit;
                    }
                    Err(err) => {
                        log::error(format!("update: cannot start {}: {err}", path.display()));
                        app.js(&format!(
                            "mdrShell.updateFailed({})",
                            json!(format!("не удалось запустить установщик: {err}"))
                        ))
                    }
                }
            }
            Event::UserEvent(UserEvent::Title(title)) => {
                app.window.set_title(&if title.is_empty() || title == APP_NAME {
                    APP_NAME.to_string()
                } else {
                    format!("{title} — {APP_NAME}")
                });
            }
            _ => {}
        }
    });
}

fn build_webview(
    window: &Window,
    web_context: &mut wry::WebContext,
    settings: &Arc<Mutex<Settings>>,
    proxy: &EventLoopProxy<UserEvent>,
) -> wry::Result<WebView> {
    let builder = WebViewBuilder::new_with_web_context(web_context)
        .with_asynchronous_custom_protocol(PROTOCOL.into(), {
            let settings = settings.clone();
            move |_id, request, responder| {
                // Scanning a folder for backlinks must not hold up the window.
                if request.uri().path() == "/_api/backlinks" {
                    let settings = settings.clone();
                    thread::spawn(move || responder.respond(server::handle(&request, &settings)));
                } else {
                    responder.respond(server::handle(&request, &settings));
                }
            }
        })
        // Only the app shell lives in the top frame; anything else goes to the browser.
        .with_navigation_handler(|url| {
            if url.starts_with(BASE_URL) || url.starts_with("mdr:") {
                return true;
            }
            open_external(&url);
            false
        })
        .with_new_window_req_handler(|url, _| {
            open_external(&url);
            wry::NewWindowResponse::Deny
        })
        .with_ipc_handler({
            let proxy = proxy.clone();
            move |req: Request<String>| {
                let _ = proxy.send_event(UserEvent::Ipc(req.into_body()));
            }
        })
        .with_drag_drop_handler({
            let proxy = proxy.clone();
            move |event| {
                if let wry::DragDropEvent::Drop { paths, .. } = event {
                    let files: Vec<_> = paths.into_iter().filter(|p| p.is_file()).collect();
                    if !files.is_empty() {
                        let _ = proxy.send_event(UserEvent::Open(files));
                    }
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
        .with_url(BASE_URL);

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let webview = builder.build(window)?;
    #[cfg(all(unix, not(target_os = "macos")))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("gtk vbox"))?
    };
    let _ = webview.focus();
    Ok(webview)
}

struct App {
    window: Window,
    webview: WebView,
    _web_context: wry::WebContext,
    menu: AppMenu,
    settings: Arc<Mutex<Settings>>,
    watched: Arc<Mutex<Vec<PathBuf>>>,
    /// Files to open once the shell page reports it is ready.
    pending: Vec<PathBuf>,
    ready: bool,
    proxy: EventLoopProxy<UserEvent>,
}

impl App {
    fn js(&self, script: &str) {
        let _ = self.webview.evaluate_script(script);
    }

    /// Asks GitHub for the latest release; `manual` says the user asked, so even
    /// "nothing new" and errors get shown.
    fn check_updates(&self, manual: bool) {
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let result = match update::check() {
                Ok(release) => json!({ "ok": true, "manual": manual, "release": release }),
                Err(error) => {
                    log::error(format!("update check: {error}"));
                    json!({ "ok": false, "manual": manual, "error": error })
                }
            };
            let _ = proxy.send_event(UserEvent::Js(format!("mdrShell.updateResult({result})")));
        });
    }

    /// The silent check at startup, at most once a day.
    fn auto_check_updates(&self) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut s = self.settings.lock().unwrap();
        if !s.update_check || now.saturating_sub(s.last_update_check) < 24 * 60 * 60 {
            return;
        }
        s.last_update_check = now;
        s.save();
        drop(s);
        self.check_updates(false);
    }

    fn install_update(&self, url: String, checksums: String) {
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let progress = |pct: u32| {
                let _ = proxy.send_event(UserEvent::Js(format!("mdrShell.updateProgress({pct})")));
            };
            let event = match update::download(&url, &checksums, progress) {
                Ok(path) => UserEvent::RunInstaller(path),
                Err(error) => {
                    log::error(format!("update download: {error}"));
                    UserEvent::Js(format!("mdrShell.updateFailed({})", json!(error)))
                }
            };
            let _ = proxy.send_event(event);
        });
    }

    fn open(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if self.ready {
            self.js(&format!("mdrShell.open({})", json!(paths)));
        } else {
            self.pending.extend(paths);
        }
    }

    fn bring_to_front(&self) {
        if self.window.is_minimized() {
            self.window.set_minimized(false);
        }
        self.window.set_focus();
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Открыть Markdown")
            .add_filter("Markdown", MARKDOWN_EXTS)
            .add_filter("Все файлы", &["*"])
            .set_parent(&self.window);
        let dir = {
            let s = self.settings.lock().unwrap();
            s.session.get(s.active).or(s.session.first()).and_then(|t| t.path.parent().map(PathBuf::from))
        };
        if let Some(dir) = dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(paths) = dialog.pick_files() {
            self.open(paths);
        }
    }

    /// Applies a settings change everywhere: disk, menu, window chrome and the page.
    fn settings_changed(&mut self, push_to_page: bool) {
        let s = self.settings.lock().unwrap().clone();
        s.save();
        self.menu.sync(&s);
        self.window.set_theme(window_theme(&s.theme));
        #[cfg(windows)]
        unsafe {
            use tao::platform::windows::WindowExtWindows;
            let _ = self.menu.menu.set_theme_for_hwnd(self.window.hwnd(), menu_theme(&s.theme));
        }
        if push_to_page {
            self.js(&format!("mdrShell.applySettings({})", json!(s)));
        }
    }

    /// Returns true when the app should exit.
    fn handle_command(&mut self, id: &str) -> bool {
        if let Some(theme) = id.strip_prefix("theme:") {
            self.settings.lock().unwrap().theme = theme.to_string();
            self.settings_changed(true);
            return false;
        }
        if let Some(width) = id.strip_prefix("width:") {
            self.settings.lock().unwrap().width = width.to_string();
            self.settings_changed(true);
            return false;
        }
        if let Some(index) = id.strip_prefix("recent:").and_then(|i| i.parse().ok()) {
            if let Some(path) = self.menu.recent_path(index) {
                self.open(vec![path]);
            }
            return false;
        }
        match id {
            "open" => self.open_dialog(),
            "new_from_clipboard" => match clipboard::read().and_then(|md| clipboard::create(&md)) {
                Ok(path) => self.open(vec![path]),
                Err(err) => self.js(&format!("mdrShell.notify('Создать из буфера', {})", json!(capitalize(&err)))),
            },
            "exit" => return true,
            "sidebar" => {
                let mut s = self.settings.lock().unwrap();
                s.sidebar = !s.sidebar;
                drop(s);
                self.settings_changed(true);
            }
            "follow" => {
                let mut s = self.settings.lock().unwrap();
                s.follow = !s.follow;
                drop(s);
                self.settings_changed(true);
            }
            "check_updates" => self.check_updates(true),
            "open_log" => {
                if let Some(path) = log::path() {
                    let _ = open::that_detached(path);
                }
            }
            "report_issue" => open_external(&log::issue_url()),
            "update_auto" => {
                let mut s = self.settings.lock().unwrap();
                s.update_check = !s.update_check;
                drop(s);
                self.settings_changed(true);
            }
            "recent_clear" => {
                self.settings.lock().unwrap().recent.clear();
                self.menu.set_recent(&[]);
                self.settings_changed(true);
            }
            "fullscreen" => {
                let full = self.window.fullscreen().is_some();
                self.window.set_fullscreen((!full).then_some(Fullscreen::Borderless(None)));
            }
            "shortcuts" => {
                rfd::MessageDialog::new()
                    .set_title("Горячие клавиши")
                    .set_description(SHORTCUTS)
                    .set_parent(&self.window)
                    .show();
            }
            "default_app" => platform::open_default_apps_settings(),
            // Everything else acts on the current tab, which only the page knows about.
            other => self.js(&format!("mdrShell.command({})", json!(other))),
        }
        false
    }

    /// Returns true when the app should exit.
    fn handle_ipc(&mut self, body: &str) -> bool {
        let Ok(msg) = serde_json::from_str::<Value>(body) else { return false };
        let str_field = |key: &str| msg.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        match msg.get("cmd").and_then(Value::as_str).unwrap_or_default() {
            "ready" => {
                self.ready = true;
                let pending = std::mem::take(&mut self.pending);
                self.open(pending);
                self.auto_check_updates();
            }
            "installUpdate" => self.install_update(str_field("url"), str_field("checksums")),
            "menu" => return self.handle_command(&str_field("id")),
            "openExternal" => open_external(&str_field("url")),
            "log" => log::error(format!("page: {}", str_field("message"))),
            "openPath" => {
                let _ = open::that_detached(str_field("path"));
            }
            "editor" => platform::open_in_editor(&PathBuf::from(str_field("path"))),
            "reveal" => platform::reveal(&PathBuf::from(str_field("path"))),
            "saveAs" => self.save_as(&PathBuf::from(str_field("path"))),
            "exportHtml" => self.export_html(&str_field("path"), &str_field("title"), &str_field("html")),
            "opened" => {
                let path = PathBuf::from(str_field("path"));
                if clipboard::is_scratch(&path) {
                    return false;
                }
                let mut s = self.settings.lock().unwrap();
                s.add_recent(&path);
                let recent = s.recent.clone();
                drop(s);
                self.menu.set_recent(&recent);
                self.settings_changed(false);
            }
            "state" => {
                let tabs: Vec<SessionTab> =
                    serde_json::from_value(msg.get("tabs").cloned().unwrap_or_default()).unwrap_or_default();
                *self.watched.lock().unwrap() = tabs.iter().map(|t| t.path.clone()).collect();
                let mut s = self.settings.lock().unwrap();
                s.session = tabs;
                s.active = msg.get("active").and_then(Value::as_u64).unwrap_or(0) as usize;
                s.save();
            }
            "settings" => {
                if let Some(patch) = msg.get("patch") {
                    self.settings.lock().unwrap().merge(patch);
                    self.settings_changed(false);
                }
            }
            _ => {}
        }
        false
    }

    /// Saves the document's source under a new name and moves its tab there. A
    /// clipboard scratch file is moved rather than copied.
    fn save_as(&self, source: &Path) {
        let fail = |err: String| {
            log::error(format!("save as {}: {err}", source.display()));
            rfd::MessageDialog::new()
                .set_title("Сохранить как")
                .set_description(format!("Не удалось сохранить файл: {err}"))
                .set_level(rfd::MessageLevel::Error)
                .set_parent(&self.window)
                .show();
        };
        let bytes = match fs::read(source) {
            Ok(bytes) => bytes,
            Err(err) => return fail(err.to_string()),
        };
        let scratch = clipboard::is_scratch(source);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Сохранить как")
            .add_filter("Markdown", &["md"])
            .add_filter("Все файлы", &["*"])
            .set_parent(&self.window);
        let name = if scratch {
            clipboard::suggested_name(&String::from_utf8_lossy(&bytes)).unwrap_or_else(|| "Документ.md".into())
        } else {
            source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        };
        let dir = if scratch { dirs::document_dir() } else { source.parent().map(PathBuf::from) };
        if let Some(dir) = dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(target) = dialog.set_file_name(name).save_file() else { return };
        if same_path(&target, source) {
            return;
        }
        if let Err(err) = fs::write(&target, &bytes) {
            return fail(err.to_string());
        }
        if scratch {
            let _ = fs::remove_file(source);
        }
        self.js(&format!("mdrShell.replacePath({}, {})", json!(source), json!(target)));
    }

    fn export_html(&self, path: &str, title: &str, article: &str) {
        let source = PathBuf::from(path);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Экспорт в HTML")
            .add_filter("HTML", &["html"])
            .set_parent(&self.window);
        if let Some(dir) = source.parent() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(stem) = source.file_stem() {
            dialog = dialog.set_file_name(format!("{}.html", stem.to_string_lossy()));
        }
        if let Some(target) = dialog.save_file() {
            if let Err(err) = fs::write(&target, server::export_page(title, article)) {
                log::error(format!("export to {}: {err}", target.display()));
                rfd::MessageDialog::new()
                    .set_title("Экспорт в HTML")
                    .set_description(format!("Не удалось сохранить файл: {err}"))
                    .set_level(rfd::MessageLevel::Error)
                    .set_parent(&self.window)
                    .show();
            }
        }
    }

    fn save_window_state(&self) {
        let mut s = self.settings.lock().unwrap();
        let maximized = self.window.is_maximized();
        if maximized || self.window.is_minimized() || self.window.fullscreen().is_some() {
            // Keep the last normal geometry so un-maximizing restores it.
            if let Some(ws) = s.window.as_mut() {
                ws.maximized = maximized;
            }
        } else if let Ok(pos) = self.window.outer_position() {
            let size = self.window.inner_size();
            s.window = Some(WindowState { x: pos.x, y: pos.y, width: size.width, height: size.height, maximized });
        }
        s.save();
    }
}

const SHORTCUTS: &str = "\
Ctrl+O — открыть файл
Ctrl+Shift+V — создать документ из буфера обмена
Ctrl+Shift+S — сохранить как
Ctrl+W — закрыть вкладку
Ctrl+Shift+T — вернуть закрытую вкладку
Ctrl+Tab / Ctrl+Shift+Tab — следующая / предыдущая вкладка
Ctrl+1…9 — перейти к вкладке
Ctrl+B — боковая панель
Ctrl+F — поиск по тексту
Ctrl+E — открыть в редакторе
Ctrl+U — исходный текст
Ctrl+P — печать / сохранить в PDF
Ctrl + / Ctrl − / Ctrl+0 — масштаб (или Ctrl + колесо)
Alt+← / Alt+→ — назад / вперёд по ссылкам
F5 — обновить
F7 / Shift+F7 — следующее / предыдущее изменение
Ctrl+Shift+R — актуализировать (снять подсветку изменений)
F11 — во весь экран";

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>() + ".").unwrap_or_default()
}

fn open_external(url: &str) {
    let allowed = ["http://", "https://", "mailto:"];
    if allowed.iter().any(|p| url.starts_with(p)) {
        let _ = open::that_detached(url);
    }
}

fn window_icon() -> Option<Icon> {
    let img = image::load_from_memory(ICON_PNG).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn window_theme(theme: &str) -> Option<Theme> {
    match theme {
        "dark" => Some(Theme::Dark),
        "light" | "sepia" => Some(Theme::Light),
        _ => None,
    }
}

#[cfg(windows)]
fn menu_theme(theme: &str) -> muda::MenuTheme {
    match theme {
        "dark" => muda::MenuTheme::Dark,
        "light" | "sepia" => muda::MenuTheme::Light,
        _ => muda::MenuTheme::Auto,
    }
}

/// Polls open documents and asks the page to re-render the ones that change on disk.
fn spawn_watcher(watched: Arc<Mutex<Vec<PathBuf>>>, proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let mut known: HashMap<PathBuf, Option<SystemTime>> = HashMap::new();
        loop {
            thread::sleep(Duration::from_millis(400));
            let paths = watched.lock().unwrap().clone();
            known.retain(|p, _| paths.iter().any(|q| same_path(p, q)));
            for path in paths {
                let mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
                match known.insert(path.clone(), mtime) {
                    Some(prev) if prev != mtime => {
                        if proxy.send_event(UserEvent::Refresh(path)).is_err() {
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }
    });
}

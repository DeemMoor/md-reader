//! Native menu bar. Shortcuts shown here are handled by the page's JS (the webview has
//! keyboard focus), so on Windows they are display-only.

use std::path::PathBuf;

use muda::accelerator::Accelerator;
use muda::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};

use crate::settings::Settings;

pub const THEMES: &[(&str, &str)] =
    &[("auto", "Как в системе"), ("light", "Светлая"), ("dark", "Тёмная"), ("sepia", "Сепия")];
pub const WIDTHS: &[(&str, &str)] = &[("full", "По ширине окна"), ("narrow", "Узкая колонка")];

pub struct AppMenu {
    pub menu: Menu,
    recent: Submenu,
    recent_paths: Vec<PathBuf>,
    sidebar: CheckMenuItem,
    follow: CheckMenuItem,
    update_auto: CheckMenuItem,
    themes: Vec<(&'static str, CheckMenuItem)>,
    widths: Vec<(&'static str, CheckMenuItem)>,
}

fn accel(s: &str) -> Option<Accelerator> {
    // macOS handles menu shortcuts natively before the webview sees them; elsewhere the
    // page handles keys, so the accelerator is only a label (Linux would double-fire).
    if cfg!(target_os = "linux") { None } else { s.parse().ok() }
}

fn item(id: &str, text: &str, shortcut: &str) -> MenuItem {
    MenuItem::with_id(id, text, true, accel(shortcut))
}

impl AppMenu {
    pub fn new(settings: &Settings) -> Self {
        let menu = Menu::new();

        #[cfg(target_os = "macos")]
        {
            let app = Submenu::with_items(
                "MD Reader",
                true,
                &[
                    &MenuItem::with_id("about", "О программе", true, None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::hide(None),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::quit(None),
                ],
            )
            .unwrap();
            let _ = menu.append(&app);
        }

        let recent = Submenu::new("Недавние", true);
        let file = Submenu::with_items(
            "&Файл",
            true,
            &[
                &item("open", "Открыть…", "CmdOrCtrl+O"),
                &recent,
                &PredefinedMenuItem::separator(),
                &item("close_tab", "Закрыть вкладку", "CmdOrCtrl+W"),
                &item("reopen_tab", "Вернуть закрытую вкладку", "CmdOrCtrl+Shift+T"),
                &PredefinedMenuItem::separator(),
                &item("editor", "Открыть в редакторе", "CmdOrCtrl+E"),
                &item("reveal", "Показать в папке", ""),
                &PredefinedMenuItem::separator(),
                &item("export_html", "Экспорт в HTML…", ""),
                &item("print", "Печать / PDF…", "CmdOrCtrl+P"),
                &PredefinedMenuItem::separator(),
                &item("exit", "Выход", "CmdOrCtrl+Q"),
            ],
        )
        .unwrap();
        let _ = menu.append(&file);

        #[cfg(target_os = "macos")]
        {
            // Without these Cmd+C / Cmd+A do nothing in a macOS webview.
            let edit = Submenu::with_items(
                "Правка",
                true,
                &[&PredefinedMenuItem::copy(None), &PredefinedMenuItem::select_all(None)],
            )
            .unwrap();
            let _ = menu.append(&edit);
        }

        let sidebar =
            CheckMenuItem::with_id("sidebar", "Боковая панель", true, settings.sidebar, accel("CmdOrCtrl+B"));
        let follow =
            CheckMenuItem::with_id("follow", "Следить за правками", true, settings.follow, None);
        let themes: Vec<_> = THEMES
            .iter()
            .map(|(id, name)| {
                (*id, CheckMenuItem::with_id(format!("theme:{id}"), *name, true, settings.theme == *id, None))
            })
            .collect();
        let widths: Vec<_> = WIDTHS
            .iter()
            .map(|(id, name)| {
                (*id, CheckMenuItem::with_id(format!("width:{id}"), *name, true, settings.width == *id, None))
            })
            .collect();
        let theme_menu = Submenu::new("Тема", true);
        for (_, it) in &themes {
            let _ = theme_menu.append(it);
        }
        let width_menu = Submenu::new("Ширина текста", true);
        for (_, it) in &widths {
            let _ = width_menu.append(it);
        }

        let view = Submenu::with_items(
            "&Вид",
            true,
            &[
                &sidebar,
                &item("show_toc", "Оглавление", ""),
                &item("show_files", "Файлы в папке", ""),
                &PredefinedMenuItem::separator(),
                &theme_menu,
                &width_menu,
                &PredefinedMenuItem::separator(),
                &item("zoom_in", "Увеличить", "CmdOrCtrl+="),
                &item("zoom_out", "Уменьшить", "CmdOrCtrl+-"),
                &item("zoom_reset", "Обычный размер", "CmdOrCtrl+0"),
                &PredefinedMenuItem::separator(),
                &item("source", "Исходный текст", "CmdOrCtrl+U"),
                &item("reload", "Обновить", "F5"),
                &item("fullscreen", "Во весь экран", "F11"),
                &PredefinedMenuItem::separator(),
                &item("next_change", "Следующее изменение", "F7"),
                &item("prev_change", "Предыдущее изменение", "Shift+F7"),
                &follow,
                &item("actualize", "Актуализировать", "CmdOrCtrl+Shift+R"),
            ],
        )
        .unwrap();
        let _ = menu.append(&view);

        let help = Submenu::new("&Справка", true);
        let _ = help.append(&item("shortcuts", "Горячие клавиши", "F1"));
        let _ = help.append(&PredefinedMenuItem::separator());
        let update_auto = CheckMenuItem::with_id(
            "update_auto",
            "Проверять обновления при запуске",
            true,
            settings.update_check,
            None,
        );
        let _ = help.append(&item("check_updates", "Проверить обновления…", ""));
        let _ = help.append(&update_auto);
        let _ = help.append(&PredefinedMenuItem::separator());
        let _ = help.append(&item("report_issue", "Сообщить о проблеме…", ""));
        let _ = help.append(&item("open_log", "Открыть журнал ошибок", ""));
        let _ = help.append(&PredefinedMenuItem::separator());
        #[cfg(windows)]
        let _ = help.append(&item("default_app", "Сделать программой для .md…", ""));
        #[cfg(not(target_os = "macos"))]
        {
            let _ = help.append(&PredefinedMenuItem::separator());
            let _ = help.append(&item("about", "О программе", ""));
        }
        let _ = menu.append(&help);

        let mut this = Self { menu, recent, recent_paths: Vec::new(), sidebar, follow, update_auto, themes, widths };
        this.set_recent(&settings.recent);
        this
    }

    pub fn set_recent(&mut self, paths: &[PathBuf]) {
        while self.recent.remove_at(0).is_some() {}
        self.recent_paths = paths.to_vec();
        if paths.is_empty() {
            let _ = self.recent.append(&MenuItem::new("Пусто", false, None));
            return;
        }
        for (i, path) in paths.iter().enumerate() {
            // "&&": a single "&" would be taken as a mnemonic marker.
            let label = path.to_string_lossy().replace('&', "&&");
            let _ = self.recent.append(&MenuItem::with_id(format!("recent:{i}"), label, true, None));
        }
        let _ = self.recent.append(&PredefinedMenuItem::separator());
        let _ = self.recent.append(&MenuItem::with_id("recent_clear", "Очистить список", true, None));
    }

    pub fn recent_path(&self, index: usize) -> Option<PathBuf> {
        self.recent_paths.get(index).cloned()
    }

    pub fn sync(&self, settings: &Settings) {
        self.sidebar.set_checked(settings.sidebar);
        self.follow.set_checked(settings.follow);
        self.update_auto.set_checked(settings.update_check);
        for (id, it) in &self.themes {
            it.set_checked(settings.theme == *id);
        }
        for (id, it) in &self.widths {
            it.set_checked(settings.width == *id);
        }
    }
}

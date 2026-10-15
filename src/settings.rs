use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 12;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// auto | light | dark | sepia
    pub theme: String,
    /// full | narrow
    pub width: String,
    pub sidebar: bool,
    /// toc | files
    pub sidebar_panel: String,
    pub sidebar_width: f64,
    pub zoom: f64,
    pub recent: Vec<PathBuf>,
    pub session: Vec<SessionTab>,
    pub active: usize,
    pub window: Option<WindowState>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionTab {
    pub path: PathBuf,
    pub scroll: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "auto".into(),
            width: "full".into(),
            sidebar: true,
            sidebar_panel: "toc".into(),
            sidebar_width: 260.0,
            zoom: 1.0,
            recent: Vec::new(),
            session: Vec::new(),
            active: 0,
            window: None,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        settings_path()
            .and_then(|p| fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            // Write-then-rename so a crash mid-write never leaves a truncated file.
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
    }

    pub fn add_recent(&mut self, path: &Path) {
        self.recent.retain(|p| !same_path(p, path));
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }

    /// Merges a partial JSON object sent by the UI (e.g. `{"sidebar": false}`).
    pub fn merge(&mut self, patch: &serde_json::Value) {
        let Ok(mut current) = serde_json::to_value(&*self) else { return };
        if let (Some(target), Some(patch)) = (current.as_object_mut(), patch.as_object()) {
            for (key, value) in patch {
                if target.contains_key(key) {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
        if let Ok(merged) = serde_json::from_value(current) {
            *self = merged;
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("MD Reader").join("settings.json"))
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").to_lowercase();
        norm(a) == norm(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_applies_known_keys_only() {
        let mut s = Settings::default();
        s.merge(&serde_json::json!({ "sidebar": false, "theme": "dark", "bogus": 1 }));
        assert!(!s.sidebar);
        assert_eq!(s.theme, "dark");
    }

    #[test]
    fn recent_is_deduplicated_and_capped() {
        let mut s = Settings::default();
        for i in 0..20 {
            s.add_recent(Path::new(&format!("C:/docs/{i}.md")));
        }
        s.add_recent(Path::new("C:/docs/5.md"));
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], Path::new("C:/docs/5.md"));
        assert_eq!(s.recent.iter().filter(|p| p.ends_with("5.md")).count(), 1);
    }
}

//! Links between documents: which files exist (for the broken-link check) and which
//! documents link to a given one (backlinks).

use std::fs;
use std::path::{Component, Path, PathBuf};

use pulldown_cmark::{Event, Options, Parser, Tag};
use serde::Serialize;

use crate::server::{is_markdown, percent_decode};

/// Limits for scanning a folder tree: enough for a project, not a whole disk.
const MAX_FILES: usize = 3000;
const MAX_DEPTH: usize = 8;
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
const MAX_PER_FILE: usize = 3;
const SKIP_DIRS: &[&str] = &["node_modules", "target", "dist", "build", "vendor", "__pycache__"];

#[derive(Serialize, Debug, PartialEq)]
pub struct Backlink {
    pub path: String,
    /// Path relative to the scanned root, for display.
    pub name: String,
    /// The text around the link, markup stripped.
    pub snippet: String,
    /// The link's own text, to highlight inside the snippet.
    pub text: String,
}

pub fn exists(paths: &[String]) -> Vec<bool> {
    paths.iter().map(|p| Path::new(p).exists()).collect()
}

/// The folder a document belongs to: the enclosing git repository, or its own folder.
pub fn project_root(doc: &Path) -> PathBuf {
    let dir = doc.parent().unwrap_or(doc);
    dir.ancestors()
        .take(MAX_DEPTH)
        .find(|d| d.join(".git").exists())
        .unwrap_or(dir)
        .to_path_buf()
}

pub fn backlinks(target: &Path) -> Vec<Backlink> {
    let root = project_root(target);
    let target = normalize(target);
    let mut files = Vec::new();
    collect(&root, 0, &mut files);
    let mut out = Vec::new();
    for file in files {
        if same(&normalize(&file), &target) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else { continue };
        let dir = file.parent().unwrap_or(&root);
        let mut found = 0;
        for (dest, text_of_link, start) in links(&text) {
            if found >= MAX_PER_FILE {
                break;
            }
            let Some(resolved) = resolve(dir, &dest) else { continue };
            if same(&resolved, &target) {
                found += 1;
                out.push(Backlink {
                    path: file.to_string_lossy().into_owned(),
                    name: file.strip_prefix(&root).unwrap_or(&file).to_string_lossy().replace('\\', "/"),
                    snippet: snippet(&text, start),
                    text: text_of_link,
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                collect(&path, depth + 1, out);
            }
        } else if kind.is_file() && is_markdown(&path) && entry.metadata().is_ok_and(|m| m.len() <= MAX_FILE_SIZE) {
            out.push(path);
            if out.len() >= MAX_FILES {
                return;
            }
        }
    }
}

/// Every link in a document: destination, link text and where it starts in the source.
fn links(text: &str) -> Vec<(String, String, usize)> {
    let mut out = Vec::new();
    let mut current: Option<(String, String, usize)> = None;
    for (event, range) in Parser::new_ext(text, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => current = Some((dest_url.into_string(), String::new(), range.start)),
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, label, _)) = current.as_mut() {
                    label.push_str(&t);
                }
            }
            Event::End(pulldown_cmark::TagEnd::Link) => out.extend(current.take()),
            _ => {}
        }
    }
    out
}

/// A link destination as a local path, or None for web links and in-page anchors.
fn resolve(dir: &Path, dest: &str) -> Option<PathBuf> {
    if dest.is_empty() || dest.starts_with('#') || dest.contains("://") || dest.starts_with("mailto:") {
        return None;
    }
    let dest = dest.split(['#', '?']).next()?;
    let dest = percent_decode(dest);
    let path = Path::new(&dest);
    Some(normalize(&if path.is_absolute() { path.to_path_buf() } else { dir.join(path) }))
}

/// Resolves "." and ".." without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn same(a: &Path, b: &Path) -> bool {
    crate::settings::same_path(a, b)
}

/// The source line holding the link, as plain text, cut to a readable length.
fn snippet(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    let line = plain(&text[start..end]);
    if line.chars().count() <= 220 {
        return line;
    }
    let cut: String = line.chars().take(220).collect();
    format!("{}…", cut.trim_end())
}

/// Strips inline Markdown from one line: links keep their text, emphasis and code marks go.
fn plain(line: &str) -> String {
    let mut out = String::new();
    for event in Parser::new_ext(line, Options::all()) {
        match event {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            _ => {}
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_found_with_their_text() {
        let found = links("See [the plan](plan.md#step-2) and [site](https://x.org).\n");
        assert_eq!(found[0].0, "plan.md#step-2");
        assert_eq!(found[0].1, "the plan");
        assert_eq!(found[1].0, "https://x.org");
    }

    #[test]
    fn destinations_resolve_to_local_paths() {
        let dir = Path::new("/docs/guide");
        assert_eq!(resolve(dir, "../plan.md#x"), Some(PathBuf::from("/docs/plan.md")));
        assert_eq!(resolve(dir, "%D0%BF%D0%BB%D0%B0%D0%BD.md"), Some(PathBuf::from("/docs/guide/план.md")));
        assert_eq!(resolve(dir, "#local"), None);
        assert_eq!(resolve(dir, "https://x.org/a.md"), None);
    }

    #[test]
    fn snippets_are_plain_text() {
        let text = "# Title\n- Details are in **[the plan](plan.md)**, see `step 2`.\n";
        assert_eq!(snippet(text, text.find("[the").unwrap()), "Details are in the plan, see step 2.");
    }

    #[test]
    fn backlinks_are_found_in_the_folder() {
        let dir = std::env::temp_dir().join(format!("mdr-backlinks-{}", std::process::id()));
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("target.md"), "# Target\n").unwrap();
        fs::write(dir.join("a.md"), "Go to [target](target.md#top).\n").unwrap();
        fs::write(dir.join("sub").join("b.md"), "Up: [t](../target.md)\n").unwrap();
        fs::write(dir.join("c.md"), "No links here.\n").unwrap();
        let found = backlinks(&dir.join("target.md"));
        let names: Vec<_> = found.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["a.md", "sub/b.md"]);
        assert_eq!(found[0].snippet, "Go to target.");
        fs::remove_dir_all(&dir).unwrap();
    }
}

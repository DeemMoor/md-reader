//! "New from clipboard": the clipboard becomes a Markdown document in a scratch file
//! that opens like any other tab. HTML copied from a web page is converted to Markdown.

use std::fs;
use std::path::{Path, PathBuf};

use crate::settings::same_path;

const BASE_NAME: &str = "Из буфера";

/// Scratch documents live here until saved elsewhere or no longer open.
pub fn scratch_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("com.deemmoor.mdreader").join("clipboard"))
}

pub fn is_scratch(path: &Path) -> bool {
    match (scratch_dir(), path.parent()) {
        (Some(dir), Some(parent)) => same_path(&dir, parent),
        _ => false,
    }
}

/// The clipboard as Markdown.
pub fn read() -> Result<String, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let text = clipboard.get_text().unwrap_or_default();
    let html = clipboard.get().html().unwrap_or_default();
    let markdown = if is_rich(&html) { to_markdown(&html).unwrap_or(text) } else { text };
    if markdown.trim().is_empty() {
        return Err("в буфере обмена нет текста".into());
    }
    Ok(markdown)
}

/// Writes `markdown` to a new scratch file.
pub fn create(markdown: &str) -> Result<PathBuf, String> {
    let dir = scratch_dir().ok_or("не найдена папка для временных файлов")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = (1..)
        .map(|n| dir.join(if n == 1 { format!("{BASE_NAME}.md") } else { format!("{BASE_NAME} {n}.md") }))
        .find(|p| !p.exists())
        .unwrap_or_default();
    fs::write(&path, markdown).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Removes scratch files that are not in `keep` (the open tabs).
pub fn cleanup(keep: &[PathBuf]) {
    let Some(entries) = scratch_dir().and_then(|d| fs::read_dir(d).ok()) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !keep.iter().any(|k| same_path(k, &path)) {
            let _ = fs::remove_file(path);
        }
    }
}

/// A file name for "Save as" taken from the document's first heading.
pub fn suggested_name(markdown: &str) -> Option<String> {
    let heading = markdown.lines().find_map(|line| {
        let line = line.trim_start();
        let text = line.trim_start_matches('#');
        let level = line.len() - text.len();
        ((1..=6).contains(&level) && (text.is_empty() || text.starts_with(' '))).then_some(text)
    })?;
    let name: String = heading
        .trim()
        .chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { ' ' } else { c })
        .collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name: String = name.chars().take(80).collect();
    let name = name.trim_end_matches(['.', ' ']);
    (!name.is_empty()).then(|| format!("{name}.md"))
}

/// Web pages put structure into the HTML; editors (VS Code and the like) only add
/// colored spans around what is already plain Markdown text.
fn is_rich(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    ["<h1", "<h2", "<h3", "<h4", "<p>", "<p ", "<li", "<table", "<a ", "<strong", "<b>", "<em", "<i>", "<img", "<pre", "<blockquote"]
        .iter()
        .any(|tag| lower.contains(tag))
}

fn to_markdown(html: &str) -> Option<String> {
    use htmd::options::{BulletListMarker, HrStyle, LinkStyle, Options};
    let converter = htmd::HtmlToMarkdown::builder()
        .options(Options {
            hr_style: HrStyle::Dashes,
            bullet_list_marker: BulletListMarker::Dash,
            ul_bullet_spacing: 1,
            ol_number_spacing: 1,
            link_style: LinkStyle::InlinedPreferAutolinks,
            ..Default::default()
        })
        .skip_tags(vec!["script", "style", "noscript", "button", "svg"])
        .build();
    let markdown = converter.convert(html).ok()?;
    let markdown = markdown.trim();
    (!markdown.is_empty()).then(|| format!("{markdown}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_html_becomes_markdown() {
        let md = to_markdown("<h2>Title</h2><p>Some <strong>bold</strong> and <a href=\"https://x.org\">link</a>.</p><ul><li>one</li><li>two</li></ul>").unwrap();
        assert_eq!(md, "## Title\n\nSome **bold** and [link](https://x.org).\n\n- one\n- two\n");
    }

    #[test]
    fn editor_html_is_not_rich() {
        assert!(!is_rich("<div style=\"color: #d4d4d4\"><span># Title</span></div>"));
        assert!(is_rich("<meta charset='utf-8'><P class=x>text</P>"));
    }

    #[test]
    fn names_come_from_the_first_heading() {
        assert_eq!(suggested_name("text\n## План: релиз 1.0?\n").as_deref(), Some("План релиз 1.0.md"));
        assert_eq!(suggested_name("no heading"), None);
        assert_eq!(suggested_name("#   \n"), None);
    }
}

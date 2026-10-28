use std::collections::HashMap;
use std::sync::LazyLock;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use syntect::highlighting::ThemeSet;
use syntect::html::{ClassStyle, ClassedHTMLGenerator, css_for_theme_with_class_style};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

const CLASS_STYLE: ClassStyle = ClassStyle::SpacedPrefixed { prefix: "hl-" };

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

/// Highlighting CSS: light colors by default, dark ones for the dark theme
/// (explicit, or "auto" while the system is dark). Relies on CSS nesting.
pub static SYNTAX_CSS: LazyLock<String> = LazyLock::new(|| {
    let themes = ThemeSet::load_defaults();
    let light = css_for_theme_with_class_style(&themes.themes["InspiredGitHub"], CLASS_STYLE)
        .unwrap_or_default();
    let dark = css_for_theme_with_class_style(&themes.themes["base16-ocean.dark"], CLASS_STYLE)
        .unwrap_or_default();
    format!(
        "{light}\n:root[data-theme=\"dark\"] {{\n{dark}\n}}\n\
         @media (prefers-color-scheme: dark) {{\n:root[data-theme=\"auto\"] {{\n{dark}\n}}\n}}"
    )
});

pub fn markdown_to_html(src: &str) -> String {
    let src = strip_bom(src);
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_GFM
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_MATH;

    let mut events: Vec<Event> = Vec::new();
    let mut code: Option<(String, String)> = None;
    let mut in_metadata = false;

    for event in Parser::new_ext(src, opts) {
        match event {
            Event::Start(Tag::MetadataBlock(_)) => in_metadata = true,
            Event::End(TagEnd::MetadataBlock(_)) => in_metadata = false,
            _ if in_metadata => {}
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .next()
                        .unwrap_or("")
                        .to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
            }
            Event::Text(text) if code.is_some() => {
                if let Some((_, buf)) = code.as_mut() {
                    buf.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, text)) = code.take() {
                    let html = if lang.eq_ignore_ascii_case("mermaid") {
                        format!(r#"<pre class="mermaid">{}</pre>"#, escape_html(&text))
                    } else {
                        highlight(&lang, &text)
                    };
                    events.push(Event::Html(html.into()));
                }
            }
            other => events.push(other),
        }
    }

    add_heading_ids(&mut events);

    let mut html = String::with_capacity(src.len() * 2);
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    html
}

/// The raw markdown, highlighted, for the "view source" mode.
pub fn source_to_html(src: &str) -> String {
    format!(
        r#"<div class="source-view">{}</div>"#,
        highlight("markdown", strip_bom(src))
    )
}

fn strip_bom(src: &str) -> &str {
    src.strip_prefix('\u{feff}').unwrap_or(src)
}

fn highlight(lang: &str, code: &str) -> String {
    let syntax = if lang.is_empty() {
        None
    } else {
        SYNTAXES.find_syntax_by_token(lang)
    };
    let body = match syntax {
        Some(syntax) => {
            let mut generator =
                ClassedHTMLGenerator::new_with_class_style(syntax, &SYNTAXES, CLASS_STYLE);
            let ok = LinesWithEndings::from(code)
                .all(|line| generator.parse_html_for_line_which_includes_newline(line).is_ok());
            if ok { generator.finalize() } else { escape_html(code) }
        }
        None => escape_html(code),
    };
    let lang_label = if lang.is_empty() {
        String::new()
    } else {
        format!(r#"<span class="code-lang">{}</span>"#, escape_html(lang))
    };
    format!(r#"<div class="code-block">{lang_label}<pre class="hl-code"><code>{body}</code></pre></div>"#)
}

/// GitHub-style anchors so that `[link](#section)` works.
fn add_heading_ids(events: &mut [Event]) {
    let mut used: HashMap<String, usize> = HashMap::new();
    for i in 0..events.len() {
        if !matches!(&events[i], Event::Start(Tag::Heading { id: None, .. })) {
            continue;
        }
        let mut text = String::new();
        for event in &events[i + 1..] {
            match event {
                Event::End(TagEnd::Heading(_)) => break,
                Event::Text(t) | Event::Code(t) => text.push_str(t),
                _ => {}
            }
        }
        let base = slugify(&text);
        let count = used.entry(base.clone()).or_insert(0);
        let slug = if *count == 0 { base } else { format!("{base}-{count}") };
        *count += 1;
        if let Event::Start(Tag::Heading { id, .. }) = &mut events[i] {
            *id = Some(slug.into());
        }
    }
}

fn slugify(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_notes_keep_pictures() {
        let html = markdown_to_html("- item\n\n<picture>\n<source media=\"(prefers-color-scheme: dark)\" srcset=\"d.gif\">\n<img src=\"l.gif\" alt=\"x\">\n</picture>\n");
        assert!(html.contains("<picture>\n<source media=\"(prefers-color-scheme: dark)\" srcset=\"d.gif\">"), "{html}");
    }

    #[test]
    fn typescript_is_highlighted() {
        for lang in ["typescript", "ts", "tsx", "toml", "rust", "markdown"] {
            let html = highlight(lang, "let x: number = 1;\n");
            assert!(html.contains("<span class=\"hl-"), "{lang}: {html}");
        }
    }

    #[test]
    fn headings_get_unique_ids() {
        let html = markdown_to_html("# Привет мир\n\n## Intro\n\n## Intro\n");
        assert!(html.contains(r#"<h1 id="привет-мир">"#));
        assert!(html.contains(r#"<h2 id="intro">"#));
        assert!(html.contains(r#"<h2 id="intro-1">"#));
    }

    #[test]
    fn mermaid_blocks_are_left_for_the_client() {
        let html = markdown_to_html("```mermaid\ngraph TD; A-->B\n```\n");
        assert!(html.contains(r#"<pre class="mermaid">graph TD; A--&gt;B"#));
    }

    #[test]
    fn math_is_marked_up() {
        let html = markdown_to_html("Inline $a^2$ and\n\n$$\\sum x$$\n");
        assert!(html.contains("math-inline"));
        assert!(html.contains("math-display"));
    }

    #[test]
    fn front_matter_is_hidden() {
        let html = markdown_to_html("---\ntitle: x\n---\n\nBody\n");
        assert!(!html.contains("title: x"));
        assert!(html.contains("Body"));
    }
}

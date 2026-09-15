//! A project's one-paragraph description, taken from its own README (or its
//! CLAUDE.md when the README has nothing to say).
//!
//! This reads the *project* folder, never Claude Code's directory, and only
//! reads it: a bounded prefix of at most two files. What comes back is plain
//! prose — every Markdown decision is made here so no shell renders Markdown.

use std::io::Read;
use std::path::Path;

/// Files tried in order. README spellings first: it is the file written for
/// people. CLAUDE.md last: it is written for Claude, but its opening paragraph
/// is often the only description a small project has.
const CANDIDATES: [&str; 5] = ["README.md", "README", "readme.md", "Readme.md", "CLAUDE.md"];

/// A README's opening paragraph is within the first few kilobytes; a
/// multi-megabyte generated file is never read whole.
const MAX_BYTES: u64 = 64 * 1024;

/// Long enough for a real sentence, short enough for three card lines.
const MAX_CHARS: usize = 180;

/// The description for the project folder at `dir`, or `None` when the folder
/// is gone or no candidate file holds a usable paragraph.
pub fn project_description(dir: &Path) -> Option<String> {
    CANDIDATES.iter().find_map(|name| {
        let mut text = String::new();
        std::fs::File::open(dir.join(name))
            .ok()?
            .take(MAX_BYTES)
            .read_to_string(&mut text)
            // A prefix cut mid-character is not valid UTF-8; fall back to the
            // lossy form rather than discarding the whole file.
            .or_else(|_| {
                let mut bytes = Vec::new();
                std::fs::File::open(dir.join(name))?
                    .take(MAX_BYTES)
                    .read_to_end(&mut bytes)?;
                text = String::from_utf8_lossy(&bytes).into_owned();
                Ok::<usize, std::io::Error>(bytes.len())
            })
            .ok()?;
        first_paragraph(&text)
    })
}

/// The first prose paragraph of a Markdown document, stripped to plain text
/// and truncated to [`MAX_CHARS`].
pub fn first_paragraph(markdown: &str) -> Option<String> {
    let mut lines = markdown.lines().peekable();

    // YAML front-matter only counts at the very top of the file.
    if lines.peek().map(|l| l.trim()) == Some("---") {
        lines.next();
        for l in lines.by_ref() {
            if l.trim() == "---" {
                break;
            }
        }
    }

    let mut paragraph: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut in_html_block = false;
    for raw in lines {
        let line = raw.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_fence = !in_fence;
            if !paragraph.is_empty() {
                break;
            }
            continue;
        }
        if in_fence {
            continue;
        }
        if in_html_block {
            if line.contains("-->") || line.starts_with("</") {
                in_html_block = false;
            }
            continue;
        }
        if line.is_empty() {
            if !paragraph.is_empty() {
                break;
            }
            continue;
        }
        if line.starts_with("<!--") {
            in_html_block = !line.contains("-->");
            continue;
        }
        if is_structural(line) {
            if !paragraph.is_empty() {
                break;
            }
            continue;
        }
        let plain = strip_inline(line);
        if plain.is_empty() {
            // A line of nothing but badges or images.
            if !paragraph.is_empty() {
                break;
            }
            continue;
        }
        paragraph.push(plain);
    }

    let joined = paragraph.join(" ");
    let collapsed = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then(|| truncate(&collapsed))
}

/// Lines that are layout rather than prose: headings, HTML, tables, lists,
/// quotes, rules and link-reference definitions.
fn is_structural(line: &str) -> bool {
    line.starts_with('#')
        || line.starts_with('<')
        || line.starts_with('|')
        || line.starts_with('>')
        || line.starts_with("- ")
        || line.starts_with("* ")
        || line.starts_with("+ ")
        || line.starts_with("===")
        || line.starts_with("---")
        || line.starts_with("***")
        || (line.starts_with('[') && line.contains("]:"))
        || line
            .split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Inline Markdown to plain text: images dropped, links reduced to their text,
/// emphasis and code markers removed, inline HTML tags removed.
fn strip_inline(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '!' && chars.get(i + 1) == Some(&'[') {
            // Image, possibly wrapped in a link: skip `![alt](url)` entirely.
            if let Some(end) = link_end(&chars, i + 1) {
                i = end;
                continue;
            }
        }
        if c == '[' {
            if let Some(close) = matching_bracket(&chars, i) {
                let inner: String = chars[i + 1..close].iter().collect();
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(paren) = find(&chars, close + 2, ')') {
                        out.push_str(&strip_inline(&inner));
                        i = paren + 1;
                        continue;
                    }
                }
            }
        }
        if c == '<' {
            if let Some(close) = find(&chars, i + 1, '>') {
                i = close + 1;
                continue;
            }
        }
        if matches!(c, '*' | '_' | '`') {
            // `_` inside a word (snake_case) is text, not emphasis.
            let inside_word = c == '_'
                && i > 0
                && chars[i - 1].is_alphanumeric()
                && chars.get(i + 1).is_some_and(|n| n.is_alphanumeric());
            if !inside_word {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out.trim().to_string()
}

/// The index just past a `[text](url)` starting at `open` (which is `[`).
fn link_end(chars: &[char], open: usize) -> Option<usize> {
    let close = matching_bracket(chars, open)?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    find(chars, close + 2, ')').map(|p| p + 1)
}

/// The `]` closing the `[` at `open`, counting nesting — a badge is an image
/// inside a link, `[![alt](src)](href)`, whose first `]` is the image's.
fn matching_bracket(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (j, &c) in chars.iter().enumerate().skip(open) {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

fn find(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&j| chars[j] == target)
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_CHARS).collect();
    let at_word = cut.rfind(' ').map_or(cut.as_str(), |i| &cut[..i]);
    format!("{}…", at_word.trim_end_matches([',', ';', ':', '.', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_readme_gives_its_first_paragraph() {
        let md =
            "# Perch\n\nA menu-bar dashboard for Claude Code.\nIt tracks sessions.\n\nMore text.";
        assert_eq!(
            first_paragraph(md).as_deref(),
            Some("A menu-bar dashboard for Claude Code. It tracks sessions.")
        );
    }

    #[test]
    fn badges_and_images_are_not_a_description() {
        let md = "# Tool\n\n[![CI](https://x/badge.svg)](https://x) ![logo](logo.png)\n\nDoes the thing.";
        assert_eq!(first_paragraph(md).as_deref(), Some("Does the thing."));
    }

    #[test]
    fn front_matter_html_and_code_are_skipped() {
        let md = "---\ntitle: x\n---\n<p align=\"center\">\n<img src=\"a.png\">\n</p>\n\n```sh\nnpm i\n```\n\nThe real words.";
        assert_eq!(first_paragraph(md).as_deref(), Some("The real words."));
    }

    #[test]
    fn inline_markdown_is_reduced_to_text() {
        let md = "A **fast** `cli` for [Claude](https://claude.ai) and snake_case names.";
        assert_eq!(
            first_paragraph(md).as_deref(),
            Some("A fast cli for Claude and snake_case names.")
        );
    }

    #[test]
    fn headings_lists_and_tables_alone_are_nothing() {
        let md = "# Title\n\n## Install\n\n- one\n- two\n\n1. first\n\n| a | b |\n|---|---|";
        assert_eq!(first_paragraph(md), None);
    }

    #[test]
    fn long_paragraphs_are_cut_on_a_word_with_an_ellipsis() {
        let md = "word ".repeat(100);
        let d = first_paragraph(&md).unwrap();
        assert!(d.ends_with('…'), "{d}");
        assert!(d.chars().count() <= MAX_CHARS + 1, "{}", d.chars().count());
        assert!(!d.contains("wor…"), "cut mid-word: {d}");
    }

    #[test]
    fn readme_wins_and_claude_md_is_the_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Notes\n\nFrom CLAUDE.md.").unwrap();
        assert_eq!(
            project_description(dir.path()).as_deref(),
            Some("From CLAUDE.md.")
        );

        std::fs::write(dir.path().join("README.md"), "# Only a heading\n").unwrap();
        assert_eq!(
            project_description(dir.path()).as_deref(),
            Some("From CLAUDE.md."),
            "a README with no prose falls through"
        );

        std::fs::write(dir.path().join("README.md"), "From the README.").unwrap();
        assert_eq!(
            project_description(dir.path()).as_deref(),
            Some("From the README.")
        );
    }

    #[test]
    fn a_missing_folder_has_no_description() {
        assert_eq!(
            project_description(Path::new("/definitely/not/here/perch")),
            None
        );
    }
}

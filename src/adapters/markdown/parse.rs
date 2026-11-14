//! Frontmatter parsing for basic-memory Markdown notes.

use crate::domain::{NewNote, Tag};

/// Parses a basic-memory note: splits the `---`-delimited frontmatter from the body and reads the
/// `title`, `type`, `permalink` and `tags` keys (tags as an inline `[a, b]` list or as following
/// `- item` lines). Pure; no I/O.
///
/// Adds `folder:<dir>`, `permalink:<p>` and `type:<t>` tags when those values are known, in that
/// order after the frontmatter tags. Values are trimmed and stripped of matching quotes; empty
/// `title`/`type`/`permalink` values are ignored.
///
/// # Arguments
/// * `text` - Whole file contents.
/// * `stem` - File name without extension; used as the title when the frontmatter has none.
/// * `folder` - Directory of the file relative to the import root, if not the root itself.
///
/// # Returns
/// The note with its title, verbatim body (frontmatter removed) and tags.
pub fn parse(text: &str, stem: &str, folder: Option<&str>) -> NewNote {
    let (front, body) = split(text);
    let (mut title, mut permalink, mut kind, mut tags) = (None, None, None, Vec::new());
    let mut in_tags = false;
    for line in front.lines() {
        if in_tags && let Some(item) = line.trim_start().strip_prefix("- ") {
            tags.push(Tag(unquote(item).to_string()));
            continue;
        }
        in_tags = false;
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = unquote(value);
        match key {
            "title" if !value.is_empty() => title = Some(value.to_string()),
            "type" if !value.is_empty() => kind = Some(value.to_string()),
            "permalink" if !value.is_empty() => permalink = Some(value.to_string()),
            "tags" => {
                in_tags = value.is_empty();
                let inline = value.trim_start_matches('[').trim_end_matches(']');
                tags.extend(split_list(inline).map(unquote).filter(|t| !t.is_empty()).map(|t| Tag(t.to_string())));
            }
            _ => {}
        }
    }
    tags.extend(folder.map(|f| Tag(format!("folder:{f}"))));
    tags.extend(permalink.map(|p| Tag(format!("permalink:{p}"))));
    tags.extend(kind.map(|k| Tag(format!("type:{k}"))));
    NewNote { title: title.unwrap_or_else(|| stem.to_string()), body: body.to_string(), tags }
}

/// Splits a file into frontmatter and body after dropping a leading UTF-8 BOM. Frontmatter
/// exists only when the text starts with a `---` line, has a closing `---` line (LF or CRLF
/// endings, trailing spaces allowed), and every non-blank line between them is a `key: value`
/// line or a `- item` list line; otherwise the leading `---` is a thematic break. Blank lines
/// after the closing line are dropped.
///
/// # Arguments
/// * `text` - Whole file contents.
///
/// # Returns
/// `(frontmatter, body)`; frontmatter is empty and body is all of `text` (without BOM) when
/// there is no valid frontmatter block.
fn split(text: &str) -> (&str, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let Some(open) = lines.next().filter(|l| l.trim_end() == "---") else { return ("", text) };
    let rest = &text[open.len()..];
    let mut start = 0;
    for line in lines {
        if line.trim_end() == "---" {
            let front = rest[..start].trim_end_matches(['\r', '\n']);
            if !front.lines().all(is_front_line) {
                break;
            }
            return (front, rest[start + line.len()..].trim_start_matches(['\r', '\n']));
        }
        start += line.len();
    }
    ("", text)
}

/// Tells whether `line` can belong to a frontmatter block: blank, a `- item` list line, or
/// `key: value` with a non-blank key.
///
/// # Arguments
/// * `line` - One frontmatter line without its line ending.
///
/// # Returns
/// `true` when the line fits the frontmatter grammar.
fn is_front_line(line: &str) -> bool {
    let line = line.trim_end();
    let item = line.trim_start();
    if line.is_empty() || item.starts_with("- ") {
        return true;
    }
    line.split_once(':').is_some_and(|(key, _)| !key.trim().is_empty())
}

/// Splits an inline list on commas that are outside double or single quotes.
///
/// # Arguments
/// * `s` - List contents without the surrounding brackets.
///
/// # Returns
/// The raw items, still quoted and untrimmed.
fn split_list(s: &str) -> impl Iterator<Item = &str> {
    let mut quote = None;
    s.split(move |c: char| {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if q == c => quote = None,
            (None, ',') => return true,
            _ => {}
        }
        false
    })
}

/// Trims whitespace and removes one pair of matching surrounding double or single quotes.
///
/// # Arguments
/// * `s` - Raw frontmatter value.
///
/// # Returns
/// The trimmed value without its enclosing quotes, borrowed from `s`.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|s| s.strip_suffix(q)) {
            return inner;
        }
    }
    s
}

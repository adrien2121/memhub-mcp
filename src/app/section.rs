//! Markdown section replacement for `EditOp::ReplaceSection`.

use crate::error::{Error, Result};

/// Parses one line as a Markdown ATX heading: 1 to 6 `#` followed by end of line, a space or
/// a tab. Does not know about code fences; [`headings`] handles those.
///
/// # Arguments
/// * `line` - One line without its line ending.
///
/// # Returns
/// The heading level and its trimmed text, or `None` when the line is not a heading.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    let rest = &line[level..];
    let ok = (1..=6).contains(&level) && (rest.is_empty() || rest.starts_with([' ', '\t']));
    ok.then(|| (level, rest.trim()))
}

/// Finds every ATX heading in `body`, skipping lines inside ``` code fences.
///
/// # Arguments
/// * `body` - Markdown note body.
///
/// # Returns
/// One tuple per heading, in order: (byte offset where the heading line starts, byte offset
/// just after its line ending, heading level 1..=6, trimmed heading text).
fn headings(body: &str) -> Vec<(usize, usize, usize, &str)> {
    let (mut out, mut pos, mut fenced) = (Vec::new(), 0, false);
    for line in body.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        if text.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if let Some((level, title)) = heading(text).filter(|_| !fenced) {
            out.push((pos, pos + line.len(), level, title));
        }
        pos += line.len();
    }
    out
}

/// Replaces the content under the first heading whose text equals `title`, up to the next
/// heading of the same or higher level (deeper subsections are replaced too). The heading line
/// itself is kept; newlines are added so the heading and the following section stay on their
/// own lines. Backs `EditOp::ReplaceSection`.
///
/// # Arguments
/// * `body` - Current Markdown note body.
/// * `title` - Heading text to match exactly after trimming, without the `#` marks.
/// * `content` - New text placed under the heading.
///
/// # Returns
/// The new body.
///
/// # Errors
/// * [`Error::InvalidInput`] - No heading outside code fences matches `title`.
pub fn replace(body: &str, title: &str, content: &str) -> Result<String> {
    let all = headings(body);
    let idx = all
        .iter()
        .position(|h| h.3 == title.trim())
        .ok_or_else(|| Error::InvalidInput(format!("no section titled {title:?}")))?;
    let (_, start, level, _) = all[idx];
    let end = all[idx + 1..].iter().find(|h| h.2 <= level).map_or(body.len(), |h| h.0);
    let mut out = String::with_capacity(body.len() + content.len() + 1);
    out.push_str(&body[..start]);
    if !body[..start].ends_with('\n') {
        out.push('\n');
    }
    out.push_str(content);
    if end < body.len() && !content.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&body[end..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::replace;

    const DOC: &str = "intro\n# A\na1\n## B\nb1\n### C\nc1\n## D\nd1\n# E\ne1\n";

    /// Proves a level-2 section is replaced up to the next level-2 heading, taking its level-3 child.
    #[test]
    fn replaces_up_to_same_level() {
        let out = replace(DOC, "B", "new").unwrap_or_default();
        assert_eq!(out, "intro\n# A\na1\n## B\nnew\n## D\nd1\n# E\ne1\n");
    }

    /// Proves a level-1 section swallows all nested subsections up to the next level-1 heading.
    #[test]
    fn top_level_swallows_subsections() {
        let out = replace(DOC, "A", "x\n").unwrap_or_default();
        assert_eq!(out, "intro\n# A\nx\n# E\ne1\n");
    }

    /// Proves the last section is replaced through the end of the body.
    #[test]
    fn last_section_runs_to_end() {
        assert_eq!(replace(DOC, "E", "z").unwrap_or_default(), "intro\n# A\na1\n## B\nb1\n### C\nc1\n## D\nd1\n# E\nz");
    }

    /// Proves a heading with no trailing newline gets one inserted before the new content.
    #[test]
    fn heading_at_eof_without_newline() {
        assert_eq!(replace("# T", "T", "body").unwrap_or_default(), "# T\nbody");
    }

    /// Proves unknown, empty, `#`-without-space and prefix-only titles are rejected.
    #[test]
    fn missing_or_partial_heading_is_invalid() {
        assert!(replace(DOC, "Z", "x").is_err());
        assert!(replace(DOC, "", "x").is_err());
        assert!(replace("#B\n", "B", "x").is_err());
        assert!(replace("# Bee\n", "B", "x").is_err());
    }

    /// Proves headings inside ``` code fences are neither matched nor treated as section ends.
    #[test]
    fn ignores_headings_in_code_fences() {
        let doc = "# A\n```\n# A\n```\n# B\n";
        assert_eq!(replace(doc, "A", "y").unwrap_or_default(), "# A\ny\n# B\n");
        assert!(replace("```\n# Q\n```\n", "Q", "y").is_err());
    }

    /// Proves only the first of two same-titled sections is replaced.
    #[test]
    fn first_match_wins() {
        assert_eq!(replace("# A\n1\n# A\n2\n", "A", "x").unwrap_or_default(), "# A\nx\n# A\n2\n");
    }
}

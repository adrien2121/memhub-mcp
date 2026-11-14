//! Interactive menu: pick the AI tools to connect and whether to add the usage rule.

use std::io::{BufRead, Result, Write};
use std::path::Path;

use super::register::overrides::{Overrides, expand};
use super::register::{Client, Target};

/// Shows the tools with found/not found, lets the user switch tools on or off by name (asking
/// for a path when a tool was not found), then asks whether to add the usage rule.
///
/// # Arguments
/// * `list` - Supported clients.
/// * `found` - Ids of clients found; these start switched on.
/// * `paths` - Overrides; paths typed for not-found tools are added here.
/// * `home` - Home directory, for `~/` in typed paths.
/// * `input` - User answers; end of input accepts the current choice.
/// * `out` - Prompts.
///
/// # Returns
/// The chosen client ids and whether to add the rule.
///
/// # Errors
/// Reading `input` or writing `out` failed.
pub(super) fn ask(
    list: &[Client],
    found: &[&'static str],
    paths: &mut Overrides,
    home: &Path,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<(Vec<&'static str>, bool)> {
    let mut selected = found.to_vec();
    loop {
        writeln!(out, "\nChoose the AI tools to connect memhub-mcp to ([x] = will connect):")?;
        for c in list {
            let mark = if selected.contains(&c.id) { "x" } else { " " };
            let state = match paths.get(c.id) {
                _ if found.contains(&c.id) => "found".to_string(),
                Some(p) => format!("path {}", p.display()),
                None => "not found".into(),
            };
            writeln!(out, "  [{mark}] {:<15} {} ({state})", c.id, c.label)?;
        }
        write!(out, "Press Enter to continue, or type a tool name to switch it on or off: ")?;
        let Some(name) = line(input, out)?.filter(|l| !l.is_empty()) else { break };
        let Some(c) = list.iter().find(|c| c.id == name) else {
            writeln!(out, "Unknown tool {name:?}.")?;
            continue;
        };
        if let Some(i) = selected.iter().position(|id| *id == c.id) {
            selected.remove(i);
        } else if found.contains(&c.id) || paths.contains_key(c.id) {
            selected.push(c.id);
        } else {
            let what = match c.target {
                Target::Cli(..) => "the path of its command",
                Target::Json(..) => "the path of its MCP config file",
            };
            write!(out, "{} was not found. Type {what}, or press Enter to cancel: ", c.label)?;
            if let Some(p) = line(input, out)?.filter(|l| !l.is_empty()) {
                paths.insert(c.id, expand(&p, home));
                selected.push(c.id);
            }
        }
    }
    write!(out, "Also tell your AI tools to use memhub-mcp? [Y/n] ")?;
    let answer = line(input, out)?.unwrap_or_default().to_lowercase();
    Ok((selected, !matches!(answer.as_str(), "n" | "no")))
}

/// Flushes `out`, then reads one trimmed line from `input`.
///
/// # Arguments
/// * `input` - Source.
/// * `out` - Prompt sink to flush first.
///
/// # Returns
/// `None` at end of input.
///
/// # Errors
/// I/O failure.
fn line(input: &mut dyn BufRead, out: &mut dyn Write) -> Result<Option<String>> {
    out.flush()?;
    let mut buf = String::new();
    Ok((input.read_line(&mut buf)? > 0).then(|| buf.trim().to_string()))
}

//! Adds a short "use memhub-mcp" rule to each AI tool's global instruction file.

use std::fs;
use std::path::{Path, PathBuf};

use super::json::with_suffix;

/// Rule telling an AI tool when to use memhub-mcp.
pub(crate) const RULE: &str = "Use the memhub-mcp tools as your long-term memory. Before starting a task, \
search notes for related past work. After solving a problem, making a decision or learning a fact, save a \
note. Edit an existing note instead of writing a duplicate.";

/// Marks the start of the managed block.
const START: &str = "<!-- memhub-mcp:start -->";
/// Marks the end of the managed block.
const END: &str = "<!-- memhub-mcp:end -->";

/// Result of adding the rule to one instruction file.
#[derive(Debug, PartialEq)]
pub(crate) enum RuleOutcome {
    /// The block was added.
    Added,
    /// An older block was replaced.
    Updated,
    /// The block was already current.
    UpToDate,
    /// The file mentions memhub-mcp outside the markers; left untouched.
    Mentioned,
    /// The file could not be written; holds the reason.
    Failed(String),
}

/// Global instruction file of a client that has one, by client id.
///
/// # Arguments
/// * `id` - Client id, e.g. `codex`.
/// * `home` - Home directory (`~`).
/// * `codex_home` - Value of `CODEX_HOME`, if set.
///
/// # Returns
/// The file path, or `None` for clients without a global instruction file.
pub(crate) fn file(id: &str, home: &Path, codex_home: Option<&Path>) -> Option<PathBuf> {
    match id {
        "claude-code" => Some(home.join(".claude/CLAUDE.md")),
        "codex" => Some(codex_home.map_or_else(|| home.join(".codex"), Path::to_path_buf).join("AGENTS.md")),
        "gemini" => Some(home.join(".gemini/GEMINI.md")),
        "omp" => Some(home.join(".omp/agent/AGENTS.md")),
        _ => None,
    }
}

/// Inserts or replaces the marker block holding [`RULE`] in `path`, creating the file when its
/// folder exists. An existing file is copied to `<file>.bak` before it is replaced.
///
/// # Arguments
/// * `path` - Instruction file.
///
/// # Returns
/// What happened to the file.
pub(crate) fn apply(path: &Path) -> RuleOutcome {
    if !path.parent().is_some_and(Path::is_dir) {
        return RuleOutcome::Failed(format!("folder of {} does not exist", path.display()));
    }
    let existing = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return RuleOutcome::Failed(format!("can't read {}: {e}", path.display())),
    };
    let text = existing.as_deref().unwrap_or("");
    let block = format!("{START}\n## memhub-mcp\n{RULE}\n{END}");
    let span = text.find(START).and_then(|s| text[s..].find(END).map(|e| (s, s + e + END.len())));
    let (outside, new, outcome) = match span {
        Some((s, e)) => {
            let new = format!("{}{block}{}", &text[..s], &text[e..]);
            let outcome = if new == text { RuleOutcome::UpToDate } else { RuleOutcome::Updated };
            (format!("{}{}", &text[..s], &text[e..]), new, outcome)
        }
        None if text.trim().is_empty() => (String::new(), format!("{block}\n"), RuleOutcome::Added),
        None => (text.to_string(), format!("{}\n\n{block}\n", text.trim_end()), RuleOutcome::Added),
    };
    if outside.contains("memhub-mcp") {
        return RuleOutcome::Mentioned;
    }
    if outcome == RuleOutcome::UpToDate {
        return outcome;
    }
    match write(path, existing.is_some(), &new) {
        Ok(()) => outcome,
        Err(e) => RuleOutcome::Failed(format!("can't write {}: {e}", path.display())),
    }
}

/// Writes `text` to `path` via a temp file and rename, first copying an existing file to `.bak`.
///
/// # Arguments
/// * `path` - Destination.
/// * `backup` - Whether `path` exists and must be backed up.
/// * `text` - New content.
///
/// # Errors
/// Any I/O failure.
fn write(path: &Path, backup: bool, text: &str) -> std::io::Result<()> {
    if backup {
        fs::copy(path, with_suffix(path, ".bak"))?;
    }
    let tmp = with_suffix(path, ".tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

/// Describes `outcome` for one report line.
///
/// # Arguments
/// * `outcome` - Result of [`apply`].
/// * `path` - The instruction file.
///
/// # Returns
/// Short text such as `rule added to ~/.gemini/GEMINI.md`.
pub(crate) fn describe(outcome: &RuleOutcome, path: &Path) -> String {
    let p = path.display();
    match outcome {
        RuleOutcome::Added => format!("rule added to {p}"),
        RuleOutcome::Updated => format!("rule updated in {p}"),
        RuleOutcome::UpToDate => format!("rule already in {p}"),
        RuleOutcome::Mentioned => format!("{p} already mentions memhub-mcp, left as is"),
        RuleOutcome::Failed(why) => format!("rule not added: {why}"),
    }
}

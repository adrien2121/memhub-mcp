//! Tests for the instruction-file rule block, using temp dirs only.

use std::fs;
use std::io::Result;
use std::path::{Path, PathBuf};

use super::rules::{RULE, RuleOutcome, apply, file};

/// Creates an empty temp dir unique to `name`.
fn temp(name: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_rules_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Proves the file is created, a rerun leaves it byte-identical with one block, and no `.bak`
/// is written for a new file.
#[test]
fn insert_then_rerun_is_idempotent() -> Result<()> {
    let path = temp("idem")?.join("AGENTS.md");
    assert_eq!(apply(&path), RuleOutcome::Added);
    let first = fs::read_to_string(&path)?;
    assert!(first.starts_with("<!-- memhub-mcp:start -->") && first.contains(RULE), "{first}");
    assert!(!path.with_extension("md.bak").exists());
    assert_eq!(apply(&path), RuleOutcome::UpToDate);
    assert_eq!(fs::read_to_string(&path)?, first);
    assert_eq!(first.matches("memhub-mcp:start").count(), 1);
    Ok(())
}

/// Proves the block is appended after existing text and the original is saved to `.bak`.
#[test]
fn appends_and_backs_up() -> Result<()> {
    let path = temp("append")?.join("GEMINI.md");
    fs::write(&path, "# Mine\nBe brief.\n")?;
    assert_eq!(apply(&path), RuleOutcome::Added);
    let text = fs::read_to_string(&path)?;
    assert!(text.starts_with("# Mine\nBe brief.\n\n<!-- memhub-mcp:start -->"), "{text}");
    assert!(text.ends_with("<!-- memhub-mcp:end -->\n"), "{text}");
    assert_eq!(fs::read_to_string(path.with_extension("md.bak"))?, "# Mine\nBe brief.\n");
    Ok(())
}

/// Proves an old block is replaced in place, keeping text around it.
#[test]
fn replaces_old_block() -> Result<()> {
    let path = temp("replace")?.join("CLAUDE.md");
    let old = "top\n<!-- memhub-mcp:start -->\nold rule\n<!-- memhub-mcp:end -->\nbottom\n";
    fs::write(&path, old)?;
    assert_eq!(apply(&path), RuleOutcome::Updated);
    let text = fs::read_to_string(&path)?;
    assert!(text.starts_with("top\n<!-- memhub-mcp:start -->") && text.ends_with("<!-- memhub-mcp:end -->\nbottom\n"));
    assert!(!text.contains("old rule") && text.contains(RULE), "{text}");
    assert_eq!(fs::read_to_string(path.with_extension("md.bak"))?, old);
    assert_eq!(apply(&path), RuleOutcome::UpToDate);
    Ok(())
}

/// Proves a file mentioning memhub-mcp outside the markers is left untouched.
#[test]
fn skips_when_already_mentioned() -> Result<()> {
    let path = temp("mention")?.join("AGENTS.md");
    fs::write(&path, "Always use memhub-mcp.\n")?;
    assert_eq!(apply(&path), RuleOutcome::Mentioned);
    assert_eq!(fs::read_to_string(&path)?, "Always use memhub-mcp.\n");
    assert!(!path.with_extension("md.bak").exists());
    Ok(())
}

/// Proves a missing folder fails without creating it.
#[test]
fn missing_folder_fails() -> Result<()> {
    let path = temp("missing")?.join("nope/AGENTS.md");
    assert!(matches!(apply(&path), RuleOutcome::Failed(why) if why.contains("does not exist")));
    assert!(!path.parent().is_some_and(Path::exists));
    Ok(())
}

/// Proves instruction file locations, including `CODEX_HOME`, and none for GUI tools.
#[test]
fn file_locations() {
    let home = Path::new("/h");
    assert_eq!(file("claude-code", home, None), Some(home.join(".claude/CLAUDE.md")));
    assert_eq!(file("codex", home, None), Some(home.join(".codex/AGENTS.md")));
    assert_eq!(file("codex", home, Some(Path::new("/c"))), Some(PathBuf::from("/c/AGENTS.md")));
    assert_eq!(file("gemini", home, None), Some(home.join(".gemini/GEMINI.md")));
    assert_eq!(file("omp", home, None), Some(home.join(".omp/agent/AGENTS.md")));
    for id in ["cursor", "claude-desktop", "vscode"] {
        assert_eq!(file(id, home, None), None);
    }
}

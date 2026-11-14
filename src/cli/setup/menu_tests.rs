//! Tests for the setup menu and connect flow: temp home, fake CLI runner, injected input.

use std::fs;
use std::io::Result;
use std::path::{Path, PathBuf};

use super::connect::{Env, Mode, connect};
use super::register::overrides::Overrides;

/// Creates a temp home unique to `name` holding `dirs`.
fn home(name: &str, dirs: &[&str]) -> Result<PathBuf> {
    let home = std::env::temp_dir().join(format!("memhub-mcp_menu_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    for dir in dirs {
        fs::create_dir_all(home.join(dir))?;
    }
    fs::create_dir_all(&home)?;
    Ok(home)
}

/// Runs [`connect`] with `input`; only `claude` exists as a fake CLI. Returns the output.
fn drive(home: &Path, mode: Mode, input: &str, calls: &mut Vec<String>) -> Result<String> {
    let env = Env { home, app_config: &home.join("appcfg"), codex_home: None, exe: Path::new("/abs/memhub-mcp") };
    let mut out = Vec::new();
    connect(&env, Overrides::new(), mode, &mut input.as_bytes(), &mut out, &mut |prog, args| {
        calls.push(format!("{prog} {}", args.join(" ")));
        (prog == "claude").then(|| (args != ["mcp", "get", "memhub-mcp"], String::new()))
    })?;
    Ok(String::from_utf8_lossy(&out).into())
}

/// Proves Enter accepts the found tools, and "yes" adds the rule to files or prints it to paste.
#[test]
fn accept_defaults_with_rules() -> Result<()> {
    let home = home("accept", &[".gemini", ".cursor", ".claude"])?;
    let mut calls = Vec::new();
    let out = drive(&home, Mode::Ask, "\n\n", &mut calls)?;
    assert!(out.contains("[x] gemini") && out.contains("[ ] omp") && out.contains("(not found)"), "{out}");
    assert!(out.contains("Also tell your AI tools to use memhub-mcp? [Y/n]"), "{out}");
    assert!(out.contains("Gemini CLI: registered; rule added to"), "{out}");
    assert!(out.contains("Claude Code: registered; rule added to"), "{out}");
    assert!(out.contains("Cursor: registered; paste the rule") && out.contains("Rule to paste into Cursor:"));
    assert!(calls.contains(&"claude mcp add --scope user memhub-mcp -- /abs/memhub-mcp".into()), "{calls:?}");
    assert!(fs::read_to_string(home.join(".gemini/GEMINI.md"))?.contains("memhub-mcp:start"));
    assert!(fs::read_to_string(home.join(".claude/CLAUDE.md"))?.contains("memhub-mcp:start"));
    assert!(!home.join(".omp").exists());
    Ok(())
}

/// Proves typing a found tool's name switches it off, and "n" adds no rule.
#[test]
fn toggle_off_and_no_rules() -> Result<()> {
    let home = home("toggle", &[".gemini", ".cursor"])?;
    let out = drive(&home, Mode::Ask, "cursor\n\nn\n", &mut Vec::new())?;
    assert!(out.contains("[ ] cursor"), "{out}");
    assert!(out.contains("Cursor: skipped") && out.contains("Gemini CLI: registered\n"), "{out}");
    assert!(!home.join(".cursor/mcp.json").exists());
    assert!(home.join(".gemini/settings.json").exists());
    assert!(!home.join(".gemini/GEMINI.md").exists() && !out.contains("Rule to paste"));
    Ok(())
}

/// Proves typing a not-found tool asks for a path, which is then used like `--path`.
#[test]
fn not_found_tool_asks_for_path() -> Result<()> {
    let home = home("path", &["custom"])?;
    let out = drive(&home, Mode::Ask, "omp\n~/custom/mcp.json\n\nn\n", &mut Vec::new())?;
    assert!(out.contains("omp was not found. Type the path of its MCP config file"), "{out}");
    assert!(out.contains("[x] omp") && out.contains("(path "), "{out}");
    assert!(out.contains("omp: registered"), "{out}");
    assert!(fs::read_to_string(home.join("custom/mcp.json"))?.contains("/abs/memhub-mcp"));
    Ok(())
}

/// Proves unknown names are reported and end of input accepts defaults, rule included.
#[test]
fn unknown_name_and_end_of_input() -> Result<()> {
    let home = home("eof", &[".omp/agent"])?;
    let out = drive(&home, Mode::Ask, "bogus\n", &mut Vec::new())?;
    assert!(out.contains("Unknown tool \"bogus\""), "{out}");
    assert!(fs::read_to_string(home.join(".omp/agent/AGENTS.md"))?.contains("memhub-mcp:start"));
    Ok(())
}

/// Proves the non-terminal defaults: no prompts, every found tool connected, rule only on request.
#[test]
fn defaults_without_menu() -> Result<()> {
    let home = home("defaults", &[".gemini", ".cursor"])?;
    let out = drive(&home, Mode::Defaults { rules: false }, "", &mut Vec::new())?;
    assert!(!out.contains("Choose") && !out.contains("[Y/n]"), "{out}");
    assert!(out.contains("Gemini CLI: registered\n") && out.contains("Cursor: registered\n"), "{out}");
    assert!(out.contains("Codex: not found") && !home.join(".gemini/GEMINI.md").exists(), "{out}");
    let out = drive(&home, Mode::Defaults { rules: true }, "", &mut Vec::new())?;
    assert!(out.contains("Gemini CLI: already registered; rule added to"), "{out}");
    Ok(())
}

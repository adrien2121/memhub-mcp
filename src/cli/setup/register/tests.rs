//! Tests for client registration, using a temp home and a fake CLI runner.

use std::fs;
use std::io::Result;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::overrides::Overrides;
use super::{Outcome, Runner, clients, register};

/// Creates an empty temp dir unique to `name`.
fn temp(name: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_register_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Registers with every client (selected or not), as `(label, outcome)`.
pub(super) fn register_all(home: &Path, paths: &Overrides, run: Runner) -> Vec<(&'static str, Outcome)> {
    let list = clients(home, &home.join("appcfg"), Path::new("/abs/memhub-mcp"), paths);
    list.iter().map(|c| (c.label, register(c, &mut *run))).collect()
}

/// Runs [`register_all`] with no client CLI installed.
fn run_all(home: &Path) -> Vec<(&'static str, Outcome)> {
    register_all(home, &Overrides::new(), &mut |_, _| None)
}

/// Outcome for `client` in `results`.
fn outcome<'a>(results: &'a [(&str, Outcome)], client: &str) -> Option<&'a Outcome> {
    results.iter().find(|(c, _)| *c == client).map(|(_, o)| o)
}

/// Reads `path` as JSON.
fn json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

/// Proves missing client dirs report not found and nothing is created.
#[test]
fn missing_dir_is_not_found() -> Result<()> {
    let home = temp("missing")?;
    let results = run_all(&home);
    assert!(results.iter().all(|(_, o)| *o == Outcome::NotFound), "{results:?}");
    assert!(!home.join(".cursor").exists());
    Ok(())
}

/// Proves the config file is created when the client dir exists but the file doesn't.
#[test]
fn creates_file_when_dir_exists() -> Result<()> {
    let home = temp("create")?;
    fs::create_dir_all(home.join(".cursor"))?;
    let results = run_all(&home);
    assert_eq!(outcome(&results, "Cursor"), Some(&Outcome::Registered));
    let v = json(&home.join(".cursor/mcp.json"))?;
    assert_eq!(v, json!({ "mcpServers": { "memhub-mcp": { "command": "/abs/memhub-mcp" } } }));
    assert!(!home.join(".cursor/mcp.json.bak").exists());
    Ok(())
}

/// Proves other servers and keys are kept and a `.bak` of the original is written.
#[test]
fn preserves_other_servers_and_backs_up() -> Result<()> {
    let home = temp("preserve")?;
    fs::create_dir_all(home.join(".gemini"))?;
    let file = home.join(".gemini/settings.json");
    let original = r#"{"theme": "dark", "mcpServers": {"other": {"command": "x"}}}"#;
    fs::write(&file, original)?;
    let results = run_all(&home);
    assert_eq!(outcome(&results, "Gemini CLI"), Some(&Outcome::Registered));
    let v = json(&file)?;
    assert_eq!(v["theme"], "dark");
    assert_eq!(v["mcpServers"]["other"], json!({ "command": "x" }));
    assert_eq!(v["mcpServers"]["memhub-mcp"], json!({ "command": "/abs/memhub-mcp" }));
    assert_eq!(fs::read_to_string(home.join(".gemini/settings.json.bak"))?, original);
    Ok(())
}

/// Proves an existing memhub-mcp entry is left byte-identical.
#[test]
fn existing_entry_untouched() -> Result<()> {
    let home = temp("existing")?;
    fs::create_dir_all(home.join(".omp/agent"))?;
    let file = home.join(".omp/agent/mcp.json");
    let original = r#"{"mcpServers": {"memhub-mcp": {"command": "mcp-gateway", "args": ["memhub"]}}}"#;
    fs::write(&file, original)?;
    let results = run_all(&home);
    assert_eq!(outcome(&results, "omp"), Some(&Outcome::Already));
    assert_eq!(fs::read_to_string(&file)?, original);
    assert!(!home.join(".omp/agent/mcp.json.bak").exists());
    Ok(())
}

/// Proves an unparseable file (JSON with comments) is left byte-identical and reported
/// failed with a manual snippet.
#[test]
fn unparseable_file_untouched() -> Result<()> {
    let home = temp("unparseable")?;
    fs::create_dir_all(home.join(".cursor"))?;
    let file = home.join(".cursor/mcp.json");
    let original = "{\n  // comment\n  \"mcpServers\": {}\n}\n";
    fs::write(&file, original)?;
    let results = run_all(&home);
    match outcome(&results, "Cursor") {
        Some(Outcome::Failed(why)) => assert!(why.contains("\"memhub-mcp\""), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(fs::read_to_string(&file)?, original);
    Ok(())
}

/// Proves VS Code uses the `servers` key and a `type: stdio` entry.
#[test]
fn vscode_uses_servers_and_stdio() -> Result<()> {
    let home = temp("vscode")?;
    let dir = home.join("appcfg/Code/User");
    fs::create_dir_all(&dir)?;
    let results = run_all(&home);
    assert_eq!(outcome(&results, "VS Code"), Some(&Outcome::Registered));
    let v = json(&dir.join("mcp.json"))?;
    assert_eq!(v, json!({ "servers": { "memhub-mcp": { "type": "stdio", "command": "/abs/memhub-mcp" } } }));
    Ok(())
}

/// Proves CLI clients: `mcp get` success means already registered; otherwise `mcp add` runs
/// with the absolute path, and "already exists" output also counts as registered.
#[test]
fn cli_clients() -> Result<()> {
    let home = temp("cli")?;
    let mut calls = Vec::new();
    let results = register_all(&home, &Overrides::new(), &mut |prog, args| {
        calls.push(format!("{prog} {}", args.join(" ")));
        match (prog, args[1]) {
            ("claude", "get") => Some((true, String::new())),
            ("codex", "get") => Some((false, "not found".into())),
            ("codex", "add") => Some((true, String::new())),
            _ => None,
        }
    });
    assert_eq!(outcome(&results, "Claude Code"), Some(&Outcome::Already));
    assert_eq!(outcome(&results, "Codex"), Some(&Outcome::Registered));
    assert!(calls.contains(&"codex mcp add memhub-mcp -- /abs/memhub-mcp".to_string()), "{calls:?}");
    assert!(!calls.iter().any(|c| c.starts_with("claude mcp add")), "{calls:?}");
    Ok(())
}

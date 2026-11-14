//! Tests for `--path` overrides, using a temp home and a fake CLI runner.

use std::fs;
use std::io::Result;
use std::path::{Path, PathBuf};

use super::Outcome;
use super::overrides::{Overrides, parse};
use super::tests::register_all;

/// Creates an empty temp dir unique to `name`.
fn temp(name: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_override_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Runs [`register_all`] with `paths`, recording CLI calls.
fn run(home: &Path, paths: &Overrides, calls: &mut Vec<String>) -> Vec<(&'static str, Outcome)> {
    register_all(home, paths, &mut |prog, args| {
        calls.push(format!("{prog} {}", args.join(" ")));
        None
    })
}

/// Outcome for `client` in `results`.
fn outcome<'a>(results: &'a [(&str, Outcome)], client: &str) -> Option<&'a Outcome> {
    results.iter().find(|(c, _)| *c == client).map(|(_, o)| o)
}

/// Proves an override file is created when its parent exists, the default location is ignored,
/// and other clients keep default detection.
#[test]
fn override_used_instead_of_default() -> Result<()> {
    let home = temp("used")?;
    fs::create_dir_all(home.join("custom"))?;
    fs::create_dir_all(home.join("appcfg/Code/User"))?;
    fs::create_dir_all(home.join(".cursor"))?;
    let paths =
        parse(&[format!("vscode={}", home.join("custom/mcp.json").display())], &home).map_err(std::io::Error::other)?;
    let results = run(&home, &paths, &mut Vec::new());
    assert_eq!(outcome(&results, "VS Code"), Some(&Outcome::Registered));
    assert!(fs::read_to_string(home.join("custom/mcp.json"))?.contains("\"servers\""));
    assert!(!home.join("appcfg/Code/User/mcp.json").exists());
    assert_eq!(outcome(&results, "Cursor"), Some(&Outcome::Registered));
    assert_eq!(outcome(&results, "Gemini CLI"), Some(&Outcome::NotFound));
    Ok(())
}

/// Proves a missing parent dir for an explicit path is reported failed, not "not found".
#[test]
fn override_parent_missing_fails() -> Result<()> {
    let home = temp("parent")?;
    let paths = parse(&["cursor=~/nope/mcp.json".into()], &home).map_err(std::io::Error::other)?;
    assert_eq!(paths["cursor"], home.join("nope/mcp.json"));
    let results = run(&home, &paths, &mut Vec::new());
    assert!(matches!(outcome(&results, "Cursor"), Some(Outcome::Failed(why)) if why.contains("does not exist")));
    assert!(!home.join("nope").exists());
    Ok(())
}

/// Proves a CLI override runs the given executable and the other CLI keeps its default name.
#[test]
fn cli_override_runs_given_executable() -> Result<()> {
    let home = temp("cli")?;
    let paths = parse(&["codex=/opt/bin/codex2".into()], &home).map_err(std::io::Error::other)?;
    let mut calls = Vec::new();
    run(&home, &paths, &mut calls);
    assert!(calls.contains(&"/opt/bin/codex2 mcp get memhub-mcp".to_string()), "{calls:?}");
    assert!(calls.contains(&"claude mcp get memhub-mcp".to_string()), "{calls:?}");
    assert!(!calls.iter().any(|c| c.starts_with("codex ")), "{calls:?}");
    Ok(())
}

/// Proves unknown client names and specs without `=` are rejected with the valid names.
#[test]
fn bad_specs_rejected() {
    for spec in ["bogus=x", "vscode"] {
        let err = parse(&[spec.into()], Path::new("/h")).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(err.contains("claude-desktop, vscode"), "{spec}: {err}");
    }
}

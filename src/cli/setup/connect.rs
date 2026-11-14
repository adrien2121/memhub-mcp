//! Connects memhub-mcp to the chosen AI tools and optionally adds the usage rule.

use std::io::{BufRead, Result, Write};
use std::path::{Path, PathBuf};

use super::menu;
use super::register::overrides::Overrides;
use super::register::rules::{self, RULE};
use super::register::{self, Outcome, Runner};

/// How the tools and the rule are chosen.
pub(super) enum Mode {
    /// Ask the user through the menu.
    Ask,
    /// Connect every found tool; add the rule when `rules` is set.
    Defaults {
        /// Add the usage rule.
        rules: bool,
    },
}

/// Locations used while connecting.
pub(super) struct Env<'a> {
    /// Home directory (`~`).
    pub(super) home: &'a Path,
    /// Per-user app config root (see [`register::clients`]).
    pub(super) app_config: &'a Path,
    /// Value of `CODEX_HOME`, if set.
    pub(super) codex_home: Option<PathBuf>,
    /// Absolute path of the memhub-mcp binary to register.
    pub(super) exe: &'a Path,
}

/// Detects the tools, picks them (menu or defaults), registers the picked ones and adds the rule
/// to their instruction files, writing one line per tool to `out`.
///
/// # Arguments
/// * `env` - Locations.
/// * `paths` - `--path` overrides; the menu may add more.
/// * `mode` - Menu or defaults.
/// * `input` - User answers (menu only).
/// * `out` - Prompts and report.
/// * `run` - Client CLI runner.
///
/// # Errors
/// Reading `input` or writing `out` failed.
pub(super) fn connect(
    env: &Env,
    mut paths: Overrides,
    mode: Mode,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    run: Runner,
) -> Result<()> {
    let list = register::clients(env.home, env.app_config, env.exe, &paths);
    let found: Vec<&str> = list.iter().filter(|c| register::found(c, run)).map(|c| c.id).collect();
    let (selected, add_rule) = match mode {
        Mode::Defaults { rules } => (found.clone(), rules),
        Mode::Ask => menu::ask(&list, &found, &mut paths, env.home, input, out)?,
    };
    let list = register::clients(env.home, env.app_config, env.exe, &paths);
    let mut paste = Vec::new();
    writeln!(out, "connecting memhub-mcp to your AI tools:")?;
    for client in &list {
        let outcome = match (selected.contains(&client.id), found.contains(&client.id)) {
            (true, _) => register::register(client, run),
            (false, true) => Outcome::Skipped,
            (false, false) => Outcome::NotFound,
        };
        let mut line = match &outcome {
            Outcome::Registered => "registered".to_string(),
            Outcome::Already => "already registered".into(),
            Outcome::NotFound => "not found".into(),
            Outcome::Skipped => "skipped".into(),
            Outcome::Failed(why) => format!("failed: {why}"),
        };
        if add_rule && matches!(outcome, Outcome::Registered | Outcome::Already) {
            match rules::file(client.id, env.home, env.codex_home.as_deref()) {
                Some(file) => line = format!("{line}; {}", rules::describe(&rules::apply(&file), &file)),
                None => {
                    line.push_str("; paste the rule below into its instructions");
                    paste.push(client.label);
                }
            }
        }
        writeln!(out, "  {}: {line}", client.label)?;
    }
    if !paste.is_empty() {
        writeln!(out, "Rule to paste into {}:\n  {RULE}", paste.join(", "))?;
    }
    writeln!(out, "Restart your AI tools to load memhub-mcp.")
}

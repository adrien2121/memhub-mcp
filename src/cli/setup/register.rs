//! Registers memhub-mcp with AI clients, never overwriting an existing entry.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

mod command;
mod json;
#[cfg(test)]
mod override_tests;
pub(super) mod overrides;
pub(super) mod rules;
#[cfg(test)]
mod rules_tests;
#[cfg(test)]
mod tests;

use command::cli;
pub(super) use command::run_command;
use json::add_json;
use overrides::Overrides;

/// Server name used in every client config.
const NAME: &str = "memhub-mcp";

/// Result of registering with one client.
#[derive(Debug, PartialEq)]
pub(super) enum Outcome {
    /// The entry was added.
    Registered,
    /// A `memhub-mcp` entry already existed and was left untouched.
    Already,
    /// The client is not installed.
    NotFound,
    /// The client is installed but the user chose not to connect it.
    Skipped,
    /// Registration failed; holds the reason and the manual step to do instead.
    Failed(String),
}

/// Runs a client CLI: `None` when the program is not on PATH, else `(success, stdout + stderr)`.
pub(super) type Runner<'a> = &'a mut dyn FnMut(&str, &[&str]) -> Option<(bool, String)>;

/// Where a client's registration goes.
pub(super) enum Target {
    /// A CLI program and the arguments that add the server.
    Cli(String, Vec<String>),
    /// A JSON config file: path, named by `--path`, servers key, server entry.
    Json(PathBuf, bool, &'static str, Value),
}

/// One supported AI client.
pub(super) struct Client {
    /// `--path` name, e.g. `claude-code`.
    pub(super) id: &'static str,
    /// Display name, e.g. `Claude Code`.
    pub(super) label: &'static str,
    /// Where registration goes.
    pub(super) target: Target,
}

/// Lists the supported clients with their targets, in a fixed order.
///
/// # Arguments
/// * `home` - Home directory (`~`).
/// * `app_config` - Per-user app config root: `~/Library/Application Support` on macOS,
///   `~/.config` on Linux, `%APPDATA%` on Windows.
/// * `exe` - Absolute path of the memhub-mcp binary to register.
/// * `paths` - User overrides: CLI executable or config file per client name.
///
/// # Returns
/// One [`Client`] per client; Claude Desktop only on macOS/Windows or when overridden.
pub(super) fn clients(home: &Path, app_config: &Path, exe: &Path, paths: &Overrides) -> Vec<Client> {
    let exe = exe.to_string_lossy().into_owned();
    let command = json!({ "command": exe });
    let cli = |id: &'static str, label, default: &str, add: &[&str]| {
        let program = paths.get(id).map_or(default.to_string(), |p| p.to_string_lossy().into());
        let args = add.iter().map(|s| s.to_string()).chain(["--".into(), exe.clone()]).collect();
        Client { id, label, target: Target::Cli(program, args) }
    };
    let file = |id: &'static str, label, default: PathBuf, key, entry: &Value| {
        let (path, explicit) = paths.get(id).map_or((default, false), |p| (p.clone(), true));
        Client { id, label, target: Target::Json(path, explicit, key, entry.clone()) }
    };
    let mut out = vec![
        cli("claude-code", "Claude Code", "claude", &["mcp", "add", "--scope", "user", NAME]),
        cli("codex", "Codex", "codex", &["mcp", "add", NAME]),
        file("gemini", "Gemini CLI", home.join(".gemini/settings.json"), "mcpServers", &command),
        file("cursor", "Cursor", home.join(".cursor/mcp.json"), "mcpServers", &command),
        file("omp", "omp", home.join(".omp/agent/mcp.json"), "mcpServers", &command),
    ];
    if cfg!(any(target_os = "macos", windows)) || paths.contains_key("claude-desktop") {
        let default = app_config.join("Claude/claude_desktop_config.json");
        out.push(file("claude-desktop", "Claude Desktop", default, "mcpServers", &command));
    }
    let vscode = json!({ "type": "stdio", "command": exe });
    out.push(file("vscode", "VS Code", app_config.join("Code/User/mcp.json"), "servers", &vscode));
    out
}

/// Whether `client` looks installed: its CLI starts (`--version`), or its config folder exists.
///
/// # Arguments
/// * `client` - Client to check.
/// * `run` - CLI runner.
///
/// # Returns
/// `true` when found.
pub(super) fn found(client: &Client, run: Runner) -> bool {
    match &client.target {
        Target::Cli(program, _) => run(program, &["--version"]).is_some(),
        Target::Json(path, ..) => path.parent().is_some_and(Path::is_dir),
    }
}

/// Registers memhub-mcp with one client.
///
/// # Arguments
/// * `client` - Client to register with.
/// * `run` - CLI runner.
///
/// # Returns
/// The outcome; failures carry the manual step.
pub(super) fn register(client: &Client, run: Runner) -> Outcome {
    match &client.target {
        Target::Cli(program, args) => cli(run, program, &args.iter().map(String::as_str).collect::<Vec<_>>()),
        Target::Json(path, explicit, key, entry) => add_json(path, *explicit, key, entry),
    }
}

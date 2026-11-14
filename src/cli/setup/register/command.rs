//! Registers memhub-mcp through a client CLI (`claude`, `codex`).

use std::process::Command;

use super::{NAME, Outcome, Runner};

/// Runs `program args` for real, capturing its output.
///
/// # Arguments
/// * `program` - Executable looked up on PATH.
/// * `args` - Its arguments.
///
/// # Returns
/// `None` when the program can't be started, else its success flag and combined output.
pub(crate) fn run_command(program: &str, args: &[&str]) -> Option<(bool, String)> {
    let out = Command::new(program).args(args).output().ok()?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Some((out.status.success(), text))
}

/// Registers through a client CLI: `<program> mcp get` first, so an existing entry is never
/// overwritten, then `<program> <add_args>`.
///
/// # Arguments
/// * `run` - CLI runner.
/// * `program` - Client CLI name.
/// * `add_args` - Arguments that add the server.
///
/// # Returns
/// The outcome; failures carry the command to run by hand.
pub(super) fn cli(run: Runner, program: &str, add_args: &[&str]) -> Outcome {
    match run(program, &["mcp", "get", NAME]) {
        None => return Outcome::NotFound,
        Some((true, _)) => return Outcome::Already,
        Some((false, _)) => {}
    }
    match run(program, add_args) {
        None => Outcome::NotFound,
        Some((true, _)) => Outcome::Registered,
        Some((false, text)) if text.contains("already exists") => Outcome::Already,
        Some((false, text)) => {
            let why = if text.trim().is_empty() { "the command failed" } else { text.trim() };
            Outcome::Failed(format!("{why}\n  run by hand: {program} {}", add_args.join(" ")))
        }
    }
}

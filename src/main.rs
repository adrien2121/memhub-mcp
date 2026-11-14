//! CLI entry point: dispatches to the relay or the daemon.
#![deny(missing_docs)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use memhub_mcp::config::Config;
use memhub_mcp::{
    cli::{backup, import, reembed, setup},
    daemon, relay,
};
use tracing_subscriber::EnvFilter;

/// Persistent note memory over MCP.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// Subcommands; none means `relay`.
#[derive(Subcommand)]
enum Command {
    /// Run the shared daemon on the Unix socket.
    Daemon,
    /// Relay stdio to the daemon, starting it if needed (default).
    Relay,
    /// Re-chunk and re-embed every note with the configured model.
    Reembed,
    /// Import basic-memory Markdown from a directory; existing titles are skipped.
    Import {
        /// Root of the basic-memory notes.
        dir: PathBuf,
    },
    /// Copy the database to a new file with `VACUUM INTO`.
    Backup {
        /// Destination; must not exist.
        path: PathBuf,
    },
    /// Download the default model and onnxruntime, then connect memhub-mcp to your AI tools.
    Setup {
        /// Use this path for a client instead of its default: `<client>=<path>`. The path is the
        /// CLI for claude-code/codex, else the config file. Clients: claude-code, codex, gemini,
        /// cursor, omp, claude-desktop, vscode. Repeatable.
        #[arg(long = "path", value_name = "CLIENT=PATH")]
        paths: Vec<String>,
        /// Without the menu, also add a short "use memhub-mcp" rule to each connected tool's
        /// instruction file. The menu asks instead.
        #[arg(long)]
        rules: bool,
        /// Don't ask: connect every tool found and add the rule.
        #[arg(long)]
        yes: bool,
    },
}

/// Initializes stderr logging (level from `RUST_LOG`, default `warn`), parses the command line,
/// loads the configuration and runs the chosen subcommand (`relay` when none is given).
///
/// The daemon and relay run on a Tokio runtime; after the relay, the runtime is shut down in the
/// background because its stdin reader may block forever. Any error is printed to stderr as
/// `memhub-mcp: <error>`.
///
/// # Returns
/// [`ExitCode::SUCCESS`] when the subcommand succeeds, [`ExitCode::FAILURE`] otherwise.
fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .init();
    let cli = Cli::parse();
    let result = Config::load().and_then(|config| {
        let runtime = tokio::runtime::Runtime::new()?;
        match cli.command.unwrap_or(Command::Relay) {
            Command::Daemon => runtime.block_on(daemon::run(config)),
            Command::Relay => {
                // The stdin read sits on a blocking thread that may never return; don't wait for it.
                let result = runtime.block_on(relay::run(config));
                runtime.shutdown_background();
                result
            }
            Command::Reembed => reembed::run(&config),
            Command::Import { dir } => import::run(&config, &dir),
            Command::Backup { path } => backup::run(&config, &path),
            Command::Setup { paths, rules, yes } => setup::run(&config, &paths, rules, yes),
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("memhub-mcp: {e}");
            ExitCode::FAILURE
        }
    }
}

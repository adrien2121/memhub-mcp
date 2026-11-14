//! Daemon lifecycle: database lock file, local socket listener, idle exit.

mod accept;
mod idle;
mod lock;
mod session;

use std::time::Duration;

use interprocess::local_socket::traits::tokio::Listener as _;
use tokio::sync::watch;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::ipc;
use crate::wiring;

pub use lock::lock;
use session::{serve, shutdown_signal};

/// Runs the shared daemon: takes the database lock, builds the MCP server, accepts local-socket
/// connections and serves each as its own MCP session until a shutdown signal (SIGINT/SIGTERM on
/// Unix, Ctrl-C on Windows) arrives or no client has been connected for the idle timeout.
///
/// On exit it joins the background embedding thread on a blocking task, so at most the chunk in
/// flight finishes before the model and runtime are torn down. The lock file is released when
/// this function returns.
///
/// # Arguments
/// * `config` - Loaded configuration; supplies the socket path, database and lock paths, and the
///   optional idle timeout `idle_exit_secs` (seconds; `None` means never exit while idle).
///
/// # Errors
/// * [`Error::Io`] - The database lock is held by another process (kind `ResourceBusy`), the lock
///   file or socket can't be created or bound, or the shutdown signal handlers can't be installed.
/// * [`Error::InvalidInput`] - Another daemon already answers on the configured socket.
/// * [`Error::Storage`] - The database can't be opened (reported by [`wiring::build`]).
/// * [`Error::Embedder`] - The background embedding thread panicked while being joined; other
///   errors reported by [`wiring::build`] are passed through.
pub async fn run(config: Config) -> Result<()> {
    let _lock = lock(&config)?;
    if ipc::connect(&config.socket_path).await.is_ok() {
        let socket = config.socket_path.display();
        let msg = format!("another daemon already serves {socket}; stop it and retry");
        return Err(Error::InvalidInput(msg));
    }
    let (server, drainer) = wiring::build(&config)?;
    let listener = ipc::listen(&config.socket_path)?;
    tracing::info!(socket = %config.socket_path.display(), "daemon listening");
    let shutdown = shutdown_signal()?;
    tokio::pin!(shutdown);
    let (clients, mut count) = watch::channel(0usize);
    let idle = config.idle_exit_secs.map(Duration::from_secs);
    let mut backoff = accept::Backoff::default();
    loop {
        let idle_wait = async {
            match idle {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending().await,
            }
        };
        let armed = *count.borrow() == 0;
        let changed = async {
            let _ = count.changed().await;
        };
        let accepted = accept::accept_with_backoff(&mut backoff, || listener.accept());
        match idle::next_event(accepted, changed, &mut shutdown, idle_wait, armed).await {
            idle::Event::Conn(stream) => serve(server.clone(), stream, clients.clone()),
            idle::Event::Changed if *count.borrow() == 0 => checkpoint(&config).await,
            idle::Event::Changed => {}
            idle::Event::Exit => break,
        }
    }
    // Stop the embedder thread before the runtime and the model are torn down; it finishes at
    // most the chunk in flight.
    tokio::task::spawn_blocking(move || drop(drainer)).await.map_err(|e| Error::Embedder(e.to_string()))?;
    checkpoint(&config).await;
    Ok(())
}

/// Merges the WAL into the database file and truncates it, logging failure instead of
/// returning it: a skipped checkpoint costs disk space, never data.
///
/// # Arguments
/// * `config` - supplies the database path.
async fn checkpoint(config: &Config) {
    let path = config.db_path.clone();
    match tokio::task::spawn_blocking(move || crate::adapters::sqlite::schema::checkpoint_truncate(&path)).await {
        Ok(Ok(())) => tracing::debug!("WAL checkpointed and truncated"),
        Ok(Err(e)) => tracing::warn!(error = %e, "WAL checkpoint skipped"),
        Err(e) => tracing::warn!(error = %e, "WAL checkpoint task failed"),
    }
}

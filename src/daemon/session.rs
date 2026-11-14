//! Per-connection MCP sessions and the OS shutdown signal the daemon waits on.

use interprocess::local_socket::tokio::Stream;
use rmcp::ServiceExt;
use tokio::sync::watch;

use crate::error::Result;
use crate::mcp::server::MemoryServer;
use crate::mcp::session::Session;

/// Installs SIGTERM and SIGINT handlers and returns a future that resolves when either arrives.
///
/// Handlers are installed eagerly so a signal sent before the future is first polled is not lost.
///
/// # Returns
/// A future that completes on the first SIGTERM or SIGINT.
///
/// # Errors
/// * [`crate::error::Error::Io`] - A signal handler can't be registered.
#[cfg(unix)]
pub(super) fn shutdown_signal() -> Result<impl Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    })
}

/// Returns a future that resolves on Ctrl-C (or a console close event); a failure to listen for
/// the event also resolves it.
///
/// # Returns
/// A future that completes when the console interrupt arrives.
///
/// # Errors
/// Never fails; the `Result` keeps the signature identical to the Unix version.
#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)]
pub(super) fn shutdown_signal() -> Result<impl Future<Output = ()>> {
    Ok(async {
        let _ = tokio::signal::ctrl_c().await;
    })
}

/// Serves one accepted connection as its own MCP session on a spawned Tokio task, returning
/// immediately.
///
/// Increments the shared client count before spawning and decrements it when the session ends, so
/// the daemon's idle timer only runs while no client is connected. Session init failures and
/// abnormal endings are logged as warnings, never propagated.
///
/// # Arguments
/// * `server` - The MCP server clone that handles this session's requests.
/// * `stream` - The accepted local-socket connection, split into read and write halves.
/// * `clients` - Count of live sessions, watched by the daemon loop for idle exit.
pub(super) fn serve(server: MemoryServer, stream: Stream, clients: watch::Sender<usize>) {
    clients.send_modify(|n| *n += 1);
    tokio::spawn(async move {
        let (read, write) = tokio::io::split(stream);
        match server.serve(Session::new(read, write)).await {
            Ok(running) => {
                if let Err(e) = running.waiting().await {
                    tracing::warn!("session ended abnormally: {e}");
                }
            }
            Err(e) => tracing::warn!("session init failed: {e}"),
        }
        clients.send_modify(|n| *n -= 1);
    });
}

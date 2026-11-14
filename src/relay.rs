//! Stdio relay: forwards MCP traffic to the daemon socket, starting the daemon if needed.

use interprocess::local_socket::tokio::Stream;
#[cfg(unix)]
use tokio::io::AsyncWriteExt;

use crate::config::Config;
#[cfg(doc)]
use crate::error::Error;
use crate::error::Result;
use crate::ipc;

#[cfg(any(windows, test))]
mod drain;
#[cfg(any(windows, test))]
mod pending;
mod spawn;
#[cfg(test)]
mod tests;

/// Runs the stdio relay: connects to the daemon (spawning it if no socket answers) and copies
/// stdin to the daemon and the daemon's replies to stdout until both sides close.
///
/// # Arguments
/// * `config` - Supplies `socket_path`, the daemon's local socket or named pipe.
///
/// # Returns
/// `Ok(())` once stdin and the daemon stream have both finished.
///
/// # Errors
/// * [`Error::Io`] - The daemon cannot be spawned, does not accept connections within 15 s, or
///   copying between stdio and the socket fails.
pub async fn run(config: Config) -> Result<()> {
    pump(connect(&config).await?).await?;
    Ok(())
}

/// Unix pump: copies stdin to the daemon; on stdin EOF half-closes the socket so the daemon
/// finishes replying, then drains the remaining replies to stdout. If the daemon hangs up
/// first, returns at once without waiting for stdin. `interprocess` makes `shutdown` a no-op,
/// so this goes through the inner tokio `UnixStream`.
///
/// # Arguments
/// * `stream` - Connected daemon socket; consumed.
///
/// # Returns
/// `Ok(())` when the daemon has closed its side and stdout is flushed.
///
/// # Errors
/// * [`std::io::Error`] - Reading stdin, writing/half-closing the socket, or writing stdout failed.
#[cfg(unix)]
async fn pump(stream: Stream) -> std::io::Result<()> {
    let Stream::UdSocket(stream) = stream;
    let (mut from_daemon, mut to_daemon) = tokio::net::UnixStream::from(stream).into_split();
    let mut stdin = tokio::io::stdin();
    let down = downstream(&mut from_daemon);
    tokio::pin!(down);
    tokio::select! {
        r = &mut down => return r,
        r = tokio::io::copy(&mut stdin, &mut to_daemon) => r?,
    };
    to_daemon.shutdown().await?;
    down.await
}

/// Windows pump: named pipes can't half-close, so after stdin EOF this keeps draining replies
/// until every forwarded request is answered or the daemon hangs up (see [`drain::relay`]).
///
/// # Arguments
/// * `stream` - Connected daemon named pipe; consumed.
///
/// # Returns
/// `Ok(())` when draining ends.
///
/// # Errors
/// * [`std::io::Error`] - Reading or writing stdio or the pipe failed.
#[cfg(windows)]
async fn pump(stream: Stream) -> std::io::Result<()> {
    let (from_daemon, to_daemon) = tokio::io::split(stream);
    drain::relay(tokio::io::stdin(), to_daemon, from_daemon, tokio::io::stdout()).await
}

/// Copies everything the daemon sends to stdout until the daemon closes, then flushes stdout.
///
/// # Arguments
/// * `from_daemon` - Read half of the daemon socket.
///
/// # Returns
/// `Ok(())` once the daemon closed its side and stdout is flushed.
///
/// # Errors
/// * [`std::io::Error`] - Reading the socket or writing stdout failed.
#[cfg(unix)]
async fn downstream(from_daemon: &mut (impl tokio::io::AsyncRead + Unpin)) -> std::io::Result<()> {
    let mut stdout = tokio::io::stdout();
    tokio::io::copy(from_daemon, &mut stdout).await?;
    stdout.flush().await
}

/// Connects to the daemon's socket, spawning a detached daemon and waiting up to
/// [`spawn::START_TIMEOUT`] for it if the first connect fails.
///
/// # Arguments
/// * `config` - Supplies `socket_path`.
///
/// # Returns
/// A connected stream to the daemon.
///
/// # Errors
/// * [`Error::Io`] - Spawning failed, or the socket did not accept within [`spawn::START_TIMEOUT`].
async fn connect(config: &Config) -> Result<Stream> {
    if let Ok(stream) = ipc::connect(&config.socket_path).await {
        return Ok(stream);
    }
    spawn::spawn_daemon()?;
    spawn::wait_for_daemon(&config.socket_path, spawn::START_TIMEOUT).await
}

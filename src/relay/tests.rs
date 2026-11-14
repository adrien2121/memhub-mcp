//! Tests for daemon start-up waiting in the stdio relay.

use std::time::{Duration, Instant};

use super::spawn::{START_TIMEOUT, wait_for_daemon};
use crate::error::{Error, Result};
use crate::ipc;
use interprocess::local_socket::traits::tokio::Listener as _;

/// Proves a daemon that takes 3.5 s to start listening (longer than the 3 s start budget the
/// relay used to have, as with a cold model load) is still reached, and promptly: a listener
/// bound after a 3.5 s delay must accept the relay's connection within 200 ms of binding.
///
/// # Returns
/// `Ok(())` when the connection arrives in time.
///
/// # Errors
/// * [`Error::Io`] - The temp dir, socket bind/accept, or the relay's connect failed.
/// * [`Error::Embedder`] - The listener task panicked or was cancelled (reused only to carry the join error).
///
/// # Panics
/// If the connection came 200 ms or more after the socket was bound.
#[tokio::test]
async fn slow_daemon_start_is_awaited() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("mm_relay_slow_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let socket = dir.join("s.sock");
    let path = socket.clone();
    let listener = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(3500)).await;
        let l = ipc::listen(&path)?;
        let up = Instant::now();
        let _ = l.accept().await?;
        std::io::Result::Ok(up.elapsed())
    });
    wait_for_daemon(&socket, START_TIMEOUT).await?;
    let waited = listener.await.map_err(|e| Error::Embedder(e.to_string()))??;
    assert!(waited < Duration::from_millis(200), "connected {waited:?} after the socket came up");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

//! Local IPC between relay and daemon: a Unix domain socket on Unix, a named pipe on Windows.
//!
//! `socket_path` is a filesystem path on Unix and a `\\.\pipe\...` path on Windows; both go
//! through the same `interprocess` local-socket API.

use std::path::Path;

use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::Stream as _;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName};

/// Connects to the daemon listening at `path`; the relay uses it to reach the daemon and the
/// daemon uses it to detect an already-running instance.
///
/// # Arguments
/// * `path` - Socket file path (Unix) or named-pipe path (Windows).
///
/// # Returns
/// The connected local-socket stream.
///
/// # Errors
/// `std::io::Error` if the path isn't a valid socket name or nothing listens there.
pub async fn connect(path: &Path) -> std::io::Result<Stream> {
    Stream::connect(path.to_fs_name::<GenericFilePath>()?).await
}

/// Binds the daemon's listener at `path`, replacing a stale Unix socket file; the file is removed
/// when the listener drops.
///
/// # Arguments
/// * `path` - Socket file path (Unix) or named-pipe path (Windows).
///
/// # Returns
/// The Tokio local-socket listener.
///
/// # Errors
/// `std::io::Error` if the path isn't a valid socket name or can't be bound.
pub fn listen(path: &Path) -> std::io::Result<Listener> {
    ListenerOptions::new()
        .name(path.to_fs_name::<GenericFilePath>()?)
        .try_overwrite(true)
        .reclaim_name(true)
        .create_tokio()
}

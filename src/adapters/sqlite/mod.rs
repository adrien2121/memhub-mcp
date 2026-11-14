//! `NoteStore` backed by SQLite (WAL, FTS5).

mod chunk_sync;
pub mod chunks;
mod embed_status;
pub mod fts;
pub mod list;
pub mod meta;
mod migrations;
mod note_edit;
pub mod notes;
mod open;
mod pool;
pub mod schema;
mod store;
mod tier;

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, RwLock};

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::vectors::Tier;

pub use pool::READ_POOL;
use pool::Readers;

/// SQLite store: one write connection behind a mutex, a bounded read pool, and the vector tier.
pub struct SqliteStore {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Readers>,
    freed: Condvar,
    tier: RwLock<Tier>,
    exact_max: usize,
    /// Exact count at which the index build is retried after a failed build; `0` when no build
    /// has failed since the tier was last loaded.
    build_at: std::sync::atomic::AtomicUsize,
    /// The running index build and the tier changes logged since it started, replayed onto the
    /// new index before the swap. Locked only while `tier` is write-locked.
    building: Mutex<tier::Build>,
}

/// Converts a rusqlite engine error into the crate error type, keeping only its message.
///
/// # Arguments
/// * `e` - the engine error, consumed.
///
/// # Returns
/// [`Error::Storage`] carrying `e`'s display text.
pub(crate) fn storage(e: rusqlite::Error) -> Error {
    Error::Storage(e.to_string())
}

/// Reads the wall clock as whole seconds since the Unix epoch.
///
/// # Returns
/// Seconds since 1970-01-01 UTC; `0` if the clock is before the epoch, `i64::MAX` if it overflows.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Maps a poisoned-mutex error (a thread panicked while holding the lock) to a storage error.
///
/// # Arguments
/// * `_` - the poison error, discarded.
///
/// # Returns
/// [`Error::Storage`] with the message "connection lock poisoned".
fn poisoned<T>(_: T) -> Error {
    Error::Storage("connection lock poisoned".into())
}

/// Opens a SQLite connection and applies the per-connection pragmas (busy timeout, WAL, foreign keys).
///
/// # Arguments
/// * `path` - database file; created if absent.
///
/// # Returns
/// The configured connection.
///
/// # Errors
/// * [`Error::Storage`] - the file cannot be opened or a pragma fails.
fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).map_err(storage)?;
    schema::configure(&conn).map_err(storage)?;
    Ok(conn)
}

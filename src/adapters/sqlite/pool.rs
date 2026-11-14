//! Bounded read-connection pool: at most [`READ_POOL`] open; callers beyond that wait.

use std::sync::MutexGuard;

use rusqlite::Connection;

use super::{SqliteStore, connect, poisoned};
use crate::error::{Error, Result};

/// Most read connections open at once; callers wait for a free one beyond that.
pub const READ_POOL: usize = 4;

/// Idle read connections plus how many are open in total.
#[derive(Default)]
pub(super) struct Readers {
    idle: Vec<Connection>,
    open: usize,
}

/// Returns its connection to the pool on drop, so a panicking reader doesn't shrink the pool.
struct Lease<'a> {
    store: &'a SqliteStore,
    conn: Option<Connection>,
}

impl Drop for Lease<'_> {
    /// Puts the leased connection back on the idle list and wakes one waiting reader. Runs on
    /// unwind too, so a panicking reader does not shrink the pool. If the pool mutex is poisoned,
    /// the connection is closed instead and its slot stays counted as open.
    fn drop(&mut self) {
        if let (Some(conn), Ok(mut pool)) = (self.conn.take(), self.store.readers.lock()) {
            pool.idle.push(conn);
            self.store.freed.notify_one();
        }
    }
}

impl SqliteStore {
    /// Runs `f` on a pooled read connection: reuses an idle one, opens a new one while fewer than
    /// [`READ_POOL`] are open, or blocks on the condition variable until a reader returns one.
    ///
    /// # Arguments
    /// * `f` - read-only work on the leased connection; the connection returns to the pool afterwards.
    ///
    /// # Returns
    /// Whatever `f` returns.
    ///
    /// # Errors
    /// * [`Error::Storage`] - the pool lock is poisoned, or a new connection cannot be opened.
    /// * Any error from `f`.
    pub(crate) fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        // The block ends the pool guard before `f` runs and before `Lease` relocks on drop.
        let conn = {
            let mut pool = self.readers.lock().map_err(poisoned)?;
            loop {
                if let Some(c) = pool.idle.pop() {
                    break c;
                }
                if pool.open < READ_POOL {
                    pool.open += 1;
                    break self.open_reader(pool)?;
                }
                pool = self.freed.wait(pool).map_err(poisoned)?;
            }
        };
        let lease = Lease { store: self, conn: Some(conn) };
        lease.conn.as_ref().map_or_else(|| Err(Error::Storage("read connection lost".into())), f)
    }

    /// Opens a new read connection after releasing the pool lock, so other readers are not blocked
    /// during the open. On failure, decrements the open count and wakes one waiter.
    ///
    /// # Arguments
    /// * `pool` - the pool guard; its slot was already reserved by the caller. Dropped immediately.
    ///
    /// # Returns
    /// A configured connection to the store's database file.
    ///
    /// # Errors
    /// * [`Error::Storage`] - the file cannot be opened or a pragma fails.
    fn open_reader(&self, pool: MutexGuard<'_, Readers>) -> Result<Connection> {
        drop(pool);
        connect(&self.path).inspect_err(|_| {
            if let Ok(mut pool) = self.readers.lock() {
                pool.open -= 1;
                self.freed.notify_one();
            }
        })
    }
}

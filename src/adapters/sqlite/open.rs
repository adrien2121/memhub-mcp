//! Opening [`SqliteStore`], tuning its vector tier, and running write transactions.

use std::path::Path;
use std::sync::{Condvar, Mutex, RwLock};

use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::{SqliteStore, connect, poisoned, schema, storage, tier};
use crate::error::Result;
impl SqliteStore {
    /// Opens the database, applies pending schema migrations, rebuilds the full-text index if the
    /// tokenizer changed, and loads the vector tier. Vector search scans every embedding until
    /// [`Self::with_exact_max_chunks`] sets a threshold.
    ///
    /// # Arguments
    /// * `path` - database file; created if absent. Read connections reopen this path later.
    /// * `tokenizer` - FTS5 tokenizer spec spliced into SQL verbatim; pass only a config-validated value.
    ///
    /// # Returns
    /// A store with one write connection and an empty read pool.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the file cannot be opened, the SQLite build lacks FTS5, a migration
    ///   fails or the schema is newer than this binary, the index rebuild fails, or the vector tier
    ///   cannot be loaded.
    pub fn open_with_tokenizer(path: &Path, tokenizer: &str) -> Result<Self> {
        let mut writer = connect(path)?;
        schema::check_fts5(&writer).map_err(storage)?;
        schema::migrate(&mut writer)?;
        schema::sync_tokenizer(&mut writer, tokenizer).map_err(storage)?;
        let tier = RwLock::new(tier::load(&writer, usize::MAX)?);
        let (path, writer) = (path.to_path_buf(), Mutex::new(writer));
        let readers = Mutex::default();
        let build_at = std::sync::atomic::AtomicUsize::new(0);
        let building = Mutex::default();
        Ok(Self { path, writer, readers, freed: Condvar::new(), tier, exact_max: usize::MAX, build_at, building })
    }

    /// Sets how many embedded chunks exact (brute-force) vector search may scan before the
    /// approximate turbovec index is used, and rebuilds the tier now under the writer lock.
    ///
    /// # Arguments
    /// * `n` - embedded-chunk count above which the turbovec index is used.
    ///
    /// # Returns
    /// The store with the new threshold applied.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the writer lock is poisoned or the tier cannot be built.
    pub fn with_exact_max_chunks(mut self, n: usize) -> Result<Self> {
        self.exact_max = n;
        let writer = self.writer.lock().map_err(poisoned)?;
        self.reload_tier(&writer)?;
        drop(writer);
        Ok(self)
    }

    /// Runs `f` in an `IMMEDIATE` transaction and commits it, then runs `after` on the result while
    /// still holding the writer lock, so no other write can run in between.
    ///
    /// # Arguments
    /// * `f` - the transactional work; an error rolls the transaction back.
    /// * `after` - post-commit work on the bare writer connection (e.g. reloading the vector tier).
    ///
    /// # Returns
    /// Whatever `after` returns.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the writer lock is poisoned, or the transaction cannot begin or commit.
    /// * Any error from `f` (nothing committed) or `after` (the write is already committed).
    pub(crate) fn write<T, U>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T>,
        after: impl FnOnce(&Connection, T) -> Result<U>,
    ) -> Result<U> {
        let mut conn = self.writer.lock().map_err(poisoned)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
        let out = f(&tx)?;
        tx.commit().map_err(storage)?;
        after(&conn, out)
    }
}

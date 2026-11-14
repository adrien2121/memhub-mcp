//! Keeps the in-memory vector tier in step with stored embeddings: counting, building and
//! candidate reads. The tier is either an exact count (search scans SQLite) or a built index.

mod index;
mod rebuild;

pub(super) use rebuild::Build;
use rebuild::Change;

use std::sync::atomic::Ordering;

use rusqlite::Connection;

use super::{SqliteStore, chunks, poisoned, storage};
use crate::domain::{ChunkId, NoteId, SearchFilter};
use crate::error::{Error, Result};
use crate::vectors::Tier;

/// Reads the ids of `note`'s chunks that have an embedding, so they can be dropped from the
/// vector tier once the note is deleted.
///
/// # Arguments
/// * `conn` - connection to read from; the write transaction when called before a delete.
/// * `note` - the note whose chunks are listed.
///
/// # Returns
/// Ids of the embedded chunks, in no particular order.
///
/// # Errors
/// * [`Error::Storage`] - the SQLite query fails.
pub fn embedded_ids(conn: &Connection, note: NoteId) -> Result<Vec<ChunkId>> {
    let mut stmt =
        conn.prepare_cached("SELECT id FROM chunks WHERE note_id = ?1 AND embedding IS NOT NULL").map_err(storage)?;
    let rows = stmt.query_map([note.0], |r| r.get(0).map(ChunkId)).map_err(storage)?;
    rows.collect::<rusqlite::Result<_>>().map_err(storage)
}

/// Counts the embedded chunks of live notes and picks the tier: an exact count up to
/// `exact_max`, otherwise a freshly built index over all of them.
///
/// # Arguments
/// * `conn` - any connection to the store.
/// * `exact_max` - largest chunk count still served by exact scans.
///
/// # Returns
/// `Tier::Exact(count)` or `Tier::Turbo(index)`.
///
/// # Errors
/// * [`Error::Storage`] - a query fails, the count is negative, or the index cannot be built.
pub fn load(conn: &Connection, exact_max: usize) -> Result<Tier> {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM chunks c JOIN notes n ON n.id = c.note_id \
             WHERE c.embedding IS NOT NULL AND n.deleted_at IS NULL",
            [],
            |r| r.get(0),
        )
        .map_err(storage)?;
    let n = usize::try_from(n).map_err(|_| Error::Storage(format!("bad count {n}")))?;
    if n <= exact_max { Ok(Tier::Exact(n)) } else { Ok(Tier::Turbo(Box::new(index::build(conn, n)?))) }
}

impl SqliteStore {
    /// Rebuilds the vector tier from the database and swaps it in under the tier write lock,
    /// discarding any index build still running (its result would be stale).
    ///
    /// # Arguments
    /// * `conn` - connection to read embeddings from, usually the writer.
    ///
    /// # Errors
    /// * [`Error::Storage`] - a tier lock is poisoned, or loading fails as in [`load`].
    pub(crate) fn reload_tier(&self, conn: &Connection) -> Result<()> {
        let tier = load(conn, self.exact_max)?;
        let mut slot = self.tier.write().map_err(poisoned)?;
        *slot = tier;
        self.building.lock().map_err(poisoned)?.cancel();
        self.build_at.store(0, Ordering::Relaxed);
        Ok(())
    }

    /// Adds a just-committed embedding to the tier under its write lock, and logs it for replay
    /// when an index build is running. Never builds: it reports when the exact count passes
    /// `exact_max` (or the back-off count), and the caller then runs [`Self::build_index`]
    /// after releasing the writer.
    ///
    /// # Arguments
    /// * `id` - the chunk that was embedded.
    /// * `vector` - its embedding.
    ///
    /// # Returns
    /// The id of the build the caller must now run with [`Self::build_index`], if one is due.
    ///
    /// # Errors
    /// * [`Error::Storage`] - a tier lock is poisoned or the index rejects the vector.
    pub(crate) fn remember(&self, id: ChunkId, vector: &[f32]) -> Result<Option<u64>> {
        let mut tier = self.tier.write().map_err(poisoned)?;
        let threshold = self.exact_max.max(self.build_at.load(Ordering::Relaxed));
        let due = tier.insert(id, vector, threshold)?;
        let mut building = self.building.lock().map_err(poisoned)?;
        if building.record(|| Change::Add(id, vector.to_vec())) {
            return Ok(None);
        }
        Ok(due.then(|| building.start()))
    }

    /// Visits nearest-chunk candidates for `query`. Unfiltered queries on a built index visit
    /// only the index's candidates; filtered queries, or the exact tier, scan every match in SQLite.
    ///
    /// # Arguments
    /// * `filter` - tag and `updated_at` range; anything but the default forces a full scan.
    /// * `query` - the query embedding.
    /// * `visit` - called with chunk id, note id and stored embedding, valid only during the call.
    ///
    /// # Errors
    /// * [`Error::Storage`] - a lock is poisoned, the pool or a query fails, a blob is malformed,
    ///   or `query` has the wrong width or a non-finite value.
    pub(crate) fn near(
        &self,
        filter: &SearchFilter,
        query: &[f32],
        visit: &mut dyn FnMut(ChunkId, NoteId, &[f32]),
    ) -> Result<()> {
        let candidates = if *filter == SearchFilter::default() {
            self.tier.read().map_err(poisoned)?.candidates(query)?
        } else {
            None
        };
        match candidates {
            Some(ids) => self.read(|c| index::visit_live(c, &ids, visit)),
            None => self.read(|c| chunks::for_each_embedding(c, filter, visit)),
        }
    }
}

#[cfg(test)]
mod tests;

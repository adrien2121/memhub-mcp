//! `NoteSearch` impl: keyword search, snippets and embedding scans.

use super::super::{SqliteStore, chunks, fts};
use crate::domain::{ChunkId, NoteId, SearchFilter};
use crate::error::Result;
use crate::ports::store::NoteSearch;

impl NoteSearch for SqliteStore {
    /// Runs an FTS5 keyword query over note titles and bodies on a pooled read connection.
    ///
    /// # Arguments
    /// * `query` - FTS5 match expression.
    /// * `filter` - optional tag and `updated_at` range restricting which notes match.
    /// * `limit` - maximum number of hits.
    ///
    /// # Returns
    /// Up to `limit` `(note id, bm25 rank)` pairs, best match first (lower rank is better).
    ///
    /// # Errors
    /// * [`Error::InvalidInput`](crate::error::Error::InvalidInput) - `limit` exceeds `i64::MAX`.
    /// * [`Error::Storage`](crate::error::Error::Storage) - the pool fails, or SQLite fails, including FTS5 syntax errors in `query`.
    fn fts_search(&self, query: &str, filter: &SearchFilter, limit: usize) -> Result<Vec<(NoteId, f64)>> {
        self.read(|c| fts::search(c, query, filter, limit))
    }

    /// Builds a highlighted excerpt of `note` around the terms of an FTS5 `query`.
    ///
    /// # Arguments
    /// * `query` - FTS5 match expression, as passed to `fts_search`.
    /// * `note` - the note to excerpt.
    ///
    /// # Returns
    /// The snippet text produced by the FTS5 `snippet` function.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the pool fails or the SQLite query fails.
    fn fts_snippet(&self, query: &str, note: NoteId) -> Result<String> {
        self.read(|c| fts::snippet(c, query, note))
    }

    /// Streams every embedded chunk of a live note matching `filter` to `visit`, read from SQLite on
    /// a pooled read connection (not from the in-memory vector index).
    ///
    /// # Arguments
    /// * `filter` - optional tag and `updated_at` range.
    /// * `visit` - called with chunk id, note id and the decoded embedding; the slice is only valid
    ///   during the call.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the pool fails, the query fails, or a stored embedding blob is malformed.
    fn for_each_embedding(&self, filter: &SearchFilter, visit: &mut dyn FnMut(ChunkId, NoteId, &[f32])) -> Result<()> {
        self.read(|c| chunks::for_each_embedding(c, filter, visit))
    }

    /// Streams likely nearest chunks for `query` to `visit`. With no filter and a built index, only
    /// the index's candidates are visited; otherwise every embedded chunk matching `filter` is.
    ///
    /// # Arguments
    /// * `filter` - optional tag and `updated_at` range; any non-default filter forces a full scan.
    /// * `query` - the query embedding.
    /// * `visit` - called with chunk id, note id and the stored embedding; the slice is only valid
    ///   during the call.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - a lock is poisoned, the pool or query fails, a blob is malformed, or
    ///   `query` has the wrong width or a non-finite value.
    fn for_each_near(
        &self,
        filter: &SearchFilter,
        query: &[f32],
        visit: &mut dyn FnMut(ChunkId, NoteId, &[f32]),
    ) -> Result<()> {
        self.near(filter, query, visit)
    }

    /// Reads one chunk's text on a pooled read connection, whether or not its note is soft-deleted.
    ///
    /// # Arguments
    /// * `chunk` - the chunk to read.
    ///
    /// # Returns
    /// The chunk text, or `None` if no chunk has that id.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the pool fails or the SQLite query fails.
    fn chunk_text(&self, chunk: ChunkId) -> Result<Option<String>> {
        self.read(|c| chunks::text(c, chunk))
    }
}

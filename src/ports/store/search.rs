//! Search reads: SQLite FTS5 keyword queries and stored-embedding scans.

use crate::domain::{ChunkId, NoteId, SearchFilter};
use crate::error::Result;

/// Keyword and vector search over stored notes; part of [`super::NoteStore`].
pub trait NoteSearch {
    /// Runs a prebuilt FTS5 (SQLite full-text search) query over live notes matching `filter`.
    ///
    /// # Arguments
    /// * `query` - FTS5 match expression, already escaped by the caller.
    /// * `filter` - Tag and update-time filters.
    /// * `limit` - Maximum number of results.
    ///
    /// # Returns
    /// `(note, bm25 score)` pairs, best match first.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn fts_search(&self, query: &str, filter: &SearchFilter, limit: usize) -> Result<Vec<(NoteId, f64)>>;

    /// Builds the FTS5 snippet of a note for a query.
    ///
    /// # Arguments
    /// * `query` - FTS5 match expression.
    /// * `note` - Note to excerpt.
    ///
    /// # Returns
    /// The snippet; empty if the note no longer matches.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn fts_snippet(&self, query: &str, note: NoteId) -> Result<String>;

    /// Calls `visit` for every stored embedding of a live note matching `filter`.
    ///
    /// # Arguments
    /// * `filter` - Tag and update-time filters.
    /// * `visit` - Receives chunk id, owning note id and the embedding.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn for_each_embedding(&self, filter: &SearchFilter, visit: &mut dyn FnMut(ChunkId, NoteId, &[f32])) -> Result<()>;

    /// Calls `visit` for every embedding that may rank among the nearest to `query`.
    ///
    /// This default visits every embedding through [`Self::for_each_embedding`]; a store with a
    /// vector index may override it to visit only the index's candidates.
    ///
    /// # Arguments
    /// * `filter` - Tag and update-time filters.
    /// * `_query` - Query embedding; unused by this default.
    /// * `visit` - Receives chunk id, owning note id and the embedding.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn for_each_near(
        &self,
        filter: &SearchFilter,
        _query: &[f32],
        visit: &mut dyn FnMut(ChunkId, NoteId, &[f32]),
    ) -> Result<()> {
        self.for_each_embedding(filter, visit)
    }

    /// Reads one chunk's text.
    ///
    /// # Arguments
    /// * `chunk` - Chunk id.
    ///
    /// # Returns
    /// The text, or `None` if the chunk no longer exists.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn chunk_text(&self, chunk: ChunkId) -> Result<Option<String>>;
}

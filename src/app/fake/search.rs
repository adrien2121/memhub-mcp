//! [`NoteSearch`] for [`FakeStore`]: substring keyword search; no embeddings.

use std::sync::atomic::Ordering;

use super::FakeStore;
use crate::domain::{ChunkId, NoteId, SearchFilter};
use crate::error::{Error, Result};
use crate::ports::store::NoteSearch;

impl NoteSearch for FakeStore {
    /// Records `query` in `last_query`, then returns notes whose body contains any `OR`-joined
    /// quoted term as a case-insensitive substring and that pass `filter`. The score is minus
    /// the note id, so lower ids rank first like a negative BM25 value.
    ///
    /// # Arguments
    /// * `query` - FTS5 expression as built by `app::query::build`.
    /// * `filter` - Tag and `updated_at` bounds (after inclusive, before exclusive).
    /// * `limit` - Most hits to return.
    ///
    /// # Returns
    /// (note, -id) pairs in insertion order.
    ///
    /// # Errors
    /// * [`Error::Storage`] - A mutex is poisoned.
    fn fts_search(&self, query: &str, filter: &SearchFilter, limit: usize) -> Result<Vec<(NoteId, f64)>> {
        *self.last_query.lock().map_err(|e| Error::Storage(e.to_string()))? = Some(query.to_owned());
        let terms: Vec<String> = query.split(" OR ").map(|t| t.trim_matches('"').to_lowercase()).collect();
        let notes = self.lock()?;
        let hits = notes.iter().filter(|n| terms.iter().any(|t| n.body.to_lowercase().contains(t)));
        let hits = hits.filter(|n| filter.tag.as_ref().is_none_or(|t| n.tags.contains(t)));
        let hits = hits.filter(|n| filter.updated_after.is_none_or(|a| n.updated_at >= a));
        let hits = hits.filter(|n| filter.updated_before.is_none_or(|b| n.updated_at < b));
        Ok(hits.map(|n| (n.id, -(n.id.0 as f64))).take(limit).collect())
    }

    /// Counts the call in `snippets` and uses the note's title as its snippet.
    ///
    /// # Arguments
    /// * `_` - Query (ignored).
    /// * `note` - Note to describe.
    ///
    /// # Returns
    /// The note's title, or an empty string if absent.
    ///
    /// # Errors
    /// * [`Error::Storage`] - The note mutex is poisoned.
    fn fts_snippet(&self, _: &str, note: NoteId) -> Result<String> {
        self.snippets.fetch_add(1, Ordering::Relaxed);
        Ok(self.lock()?.iter().find(|n| n.id == note).map(|n| n.title.clone()).unwrap_or_default())
    }

    /// Visits nothing: the fake stores no embeddings.
    ///
    /// # Arguments
    /// * `_` - Filter and visitor (both ignored).
    ///
    /// # Errors
    /// Never fails.
    fn for_each_embedding(&self, _: &SearchFilter, _: &mut dyn FnMut(ChunkId, NoteId, &[f32])) -> Result<()> {
        Ok(())
    }

    /// Finds no chunk text: the fake stores no chunks.
    ///
    /// # Arguments
    /// * `_` - Chunk id (ignored).
    ///
    /// # Returns
    /// Always `None`.
    ///
    /// # Errors
    /// Never fails.
    fn chunk_text(&self, _: ChunkId) -> Result<Option<String>> {
        Ok(None)
    }
}

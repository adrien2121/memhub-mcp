//! FTS5 keyword search and snippets.

use rusqlite::{Connection, params};

use super::storage;
use crate::domain::{NoteId, SearchFilter};
use crate::error::{Error, Result};

mod snippet;

pub use snippet::snippet;

/// Runs a prebuilt FTS5 `query` over live (not soft-deleted) notes matching `filter`, ranked by
/// BM25. Builds no snippets, so candidates past the returned page cost only their score.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `query` - prebuilt FTS5 match expression.
/// * `filter` - optional tag and `updated_at` range (after inclusive, before exclusive).
/// * `limit` - maximum number of hits.
///
/// # Returns
/// `(note id, bm25 score)` pairs, best (lowest score) first.
///
/// # Errors
/// * [`Error::InvalidInput`] - `limit` exceeds `i64::MAX`.
/// * [`Error::Storage`] - the SQLite query fails, including FTS5 syntax errors in `query`.
pub fn search(conn: &Connection, query: &str, filter: &SearchFilter, limit: usize) -> Result<Vec<(NoteId, f64)>> {
    let limit = i64::try_from(limit).map_err(|_| Error::InvalidInput("limit too large".into()))?;
    let mut stmt = conn
        .prepare_cached(
            "SELECT notes_fts.rowid, bm25(notes_fts) \
             FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid \
             LEFT JOIN note_tags t ON t.note_id = n.id AND t.tag = ?2 \
             WHERE notes_fts MATCH ?1 AND n.deleted_at IS NULL \
             AND (?2 IS NULL OR t.tag IS NOT NULL) \
             AND (?3 IS NULL OR n.updated_at >= ?3) AND (?4 IS NULL OR n.updated_at < ?4) \
             ORDER BY rank LIMIT ?5",
        )
        .map_err(storage)?;
    let tag = filter.tag.as_ref().map(|t| &t.0);
    stmt.query_map(params![query, tag, filter.updated_after, filter.updated_before, limit], |r| {
        Ok((NoteId(r.get(0)?), r.get(1)?))
    })
    .map_err(storage)?
    .collect::<rusqlite::Result<_>>()
    .map_err(storage)
}

//! Embedding progress per chunk: failed attempts and pending/failed/total counts.

use rusqlite::{Connection, OptionalExtension};

/// Failed embeddings after which a chunk stops being retried and counts as failed.
pub const MAX_EMBED_ATTEMPTS: i64 = 3;

use super::storage;
use crate::domain::{ChunkId, NoteId};
use crate::error::{Error, Result};

/// Increments the failed-attempt counter of `chunk` if it still has no embedding.
///
/// # Arguments
/// * `conn` - the write connection.
/// * `chunk` - the chunk whose embedding failed.
///
/// # Returns
/// `true` exactly when this failure brings the count to [`MAX_EMBED_ATTEMPTS`] (the chunk is now
/// failed). `false` otherwise, including when the chunk is gone or already embedded.
///
/// # Errors
/// * [`Error::Storage`] - the update fails.
pub fn record_embed_failure(conn: &Connection, chunk: ChunkId) -> Result<bool> {
    let attempts: Option<i64> = conn
        .query_row(
            "UPDATE chunks SET embed_attempts = embed_attempts + 1 WHERE id = ?1 AND embedding IS NULL \
             RETURNING embed_attempts",
            [chunk.0],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    Ok(attempts == Some(MAX_EMBED_ATTEMPTS))
}

/// Counts chunks of non-deleted notes that have no embedding and fewer than
/// [`MAX_EMBED_ATTEMPTS`] failed attempts.
///
/// # Arguments
/// * `conn` - any connection to the store.
///
/// # Returns
/// The number of chunks still waiting for an embedding.
///
/// # Errors
/// * [`Error::Storage`] - the query fails or the count is negative.
pub fn pending_count(conn: &Connection) -> Result<usize> {
    count(conn, &format!("c.embedding IS NULL AND c.embed_attempts < {MAX_EMBED_ATTEMPTS} AND"))
}

/// Counts chunks of non-deleted notes that have no embedding after [`MAX_EMBED_ATTEMPTS`] or more
/// failed attempts.
///
/// # Arguments
/// * `conn` - any connection to the store.
///
/// # Returns
/// The number of chunks that are no longer retried.
///
/// # Errors
/// * [`Error::Storage`] - the query fails or the count is negative.
pub fn failed_count(conn: &Connection) -> Result<usize> {
    count(conn, &format!("c.embedding IS NULL AND c.embed_attempts >= {MAX_EMBED_ATTEMPTS} AND"))
}

/// Counts all chunks of non-deleted notes, embedded or not.
///
/// # Arguments
/// * `conn` - any connection to the store.
///
/// # Returns
/// The total chunk count.
///
/// # Errors
/// * [`Error::Storage`] - the query fails or the count is negative.
pub fn chunk_count(conn: &Connection) -> Result<usize> {
    count(conn, "")
}

/// Counts chunks joined to non-deleted notes that also match an extra SQL condition.
///
/// # Arguments
/// * `conn` - any connection to the store.
/// * `cond` - SQL condition spliced verbatim before the deleted-note filter. It must end with
///   `AND`, or be empty. Only pass internal constants.
///
/// # Returns
/// The matching row count.
///
/// # Errors
/// * [`Error::Storage`] - the query fails, or the count does not fit `usize`.
fn count(conn: &Connection, cond: &str) -> Result<usize> {
    let sql =
        format!("SELECT count(*) FROM chunks c JOIN notes n ON n.id = c.note_id WHERE {cond} n.deleted_at IS NULL");
    let n: i64 = conn.query_row(&sql, [], |r| r.get(0)).map_err(storage)?;
    usize::try_from(n).map_err(|_| Error::Storage(format!("bad count {n}")))
}

/// Whether `c` carries no text for a tokenizer: Unicode white space, NUL, zero-width space, word
/// joiner, or byte-order mark.
fn is_blank(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\0' | '\u{200b}' | '\u{2060}' | '\u{feff}')
}

/// Lists live notes with a non-blank body but no chunk rows, e.g. written while no embedder was
/// configured, so they can be chunked once one is. Bodies made only of [`is_blank`] characters
/// give no chunks, so they are left out; otherwise they would be re-read on every drain.
///
/// # Arguments
/// * `conn` - any connection to the store.
///
/// # Returns
/// Their ids in ascending order.
///
/// # Errors
/// * [`Error::Storage`] - the query fails.
pub fn chunkless_notes(conn: &Connection) -> Result<Vec<NoteId>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT n.id, n.body FROM notes n WHERE n.deleted_at IS NULL AND trim(n.body) <> '' \
             AND NOT EXISTS (SELECT 1 FROM chunks c WHERE c.note_id = n.id) ORDER BY n.id",
        )
        .map_err(storage)?;
    let mut ids = Vec::new();
    let mut rows = stmt.query([]).map_err(storage)?;
    while let Some(row) = rows.next().map_err(storage)? {
        let body: String = row.get(1).map_err(storage)?;
        if !body.chars().all(is_blank) {
            ids.push(NoteId(row.get(0).map_err(storage)?));
        }
    }
    Ok(ids)
}

/// Clears the failed-attempt counter of every unembedded chunk, so failed chunks are retried.
///
/// # Arguments
/// * `conn` - the write connection.
///
/// # Returns
/// The number of chunks cleared.
///
/// # Errors
/// * [`Error::Storage`] - the update fails.
pub fn reset_failures(conn: &Connection) -> Result<usize> {
    conn.execute("UPDATE chunks SET embed_attempts = 0 WHERE embedding IS NULL AND embed_attempts > 0", [])
        .map_err(storage)
}

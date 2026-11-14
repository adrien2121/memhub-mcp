//! Chunk rows and their embeddings.

use rusqlite::{Connection, OptionalExtension, params, types::ValueRef};

pub use super::chunk_sync::replace;
pub use super::embed_status::{
    MAX_EMBED_ATTEMPTS, chunk_count, chunkless_notes, failed_count, pending_count, record_embed_failure, reset_failures,
};
use super::storage;
use crate::domain::{Chunk, ChunkId, NoteId, SearchFilter};
use crate::error::{Error, Result};

/// Converts a `usize` (offset, count or limit) to the `i64` SQLite stores integers as.
///
/// # Arguments
/// * `n` - the unsigned value to convert.
///
/// # Returns
/// `n` as an `i64`.
///
/// # Errors
/// * [`Error::InvalidInput`] - `n` exceeds `i64::MAX`.
pub(crate) fn to_i64(n: usize) -> Result<i64> {
    i64::try_from(n).map_err(|_| Error::InvalidInput(format!("{n} is out of range")))
}

/// Returns live notes' unembedded chunks past a cursor, under [`MAX_EMBED_ATTEMPTS`]. Read-only.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `after` - keyset cursor: only chunks with a greater id are returned.
/// * `limit` - maximum number of chunks returned.
///
/// # Returns
/// Up to `limit` chunks with ids above `after`, ordered by ascending chunk id.
///
/// # Errors
/// * [`Error::InvalidInput`] - `limit` exceeds `i64::MAX`.
/// * [`Error::Storage`] - the SQLite query fails.
pub fn pending(conn: &Connection, after: ChunkId, limit: usize) -> Result<Vec<Chunk>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT c.id, c.note_id, c.seq, c.text FROM chunks c JOIN notes n ON n.id = c.note_id \
             WHERE c.embedding IS NULL AND c.embed_attempts < ?2 AND c.id > ?3 AND n.deleted_at IS NULL \
             ORDER BY c.id LIMIT ?1",
        )
        .map_err(storage)?;
    stmt.query_map([to_i64(limit)?, MAX_EMBED_ATTEMPTS, after.0], |r| {
        Ok(Chunk { id: ChunkId(r.get(0)?), note_id: NoteId(r.get(1)?), seq: r.get(2)?, text: r.get(3)? })
    })
    .map_err(storage)?
    .collect::<rusqlite::Result<_>>()
    .map_err(storage)
}

/// Stores `vector` as an f32 little-endian blob on the chunk, only if the chunk still exists and
/// has no embedding yet; one `UPDATE` statement.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `chunk` - id of the chunk to update.
/// * `vector` - the embedding; must be finite and not all zeros, because cosine similarity is
///   undefined for a zero vector and NaN/infinite values would corrupt search scores.
///
/// # Returns
/// `true` if the row was updated; `false` for a stale write (chunk deleted or already embedded).
///
/// # Errors
/// * [`Error::InvalidInput`] - `vector` has a non-finite value or is all zeros.
/// * [`Error::Storage`] - the SQLite update fails.
pub fn set_embedding(conn: &Connection, chunk: ChunkId, vector: &[f32]) -> Result<bool> {
    if vector.iter().any(|f| !f.is_finite()) || vector.iter().all(|&f| f == 0.0) {
        return Err(Error::InvalidInput("embedding must be finite and nonzero".into()));
    }
    let blob: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
    let n = conn
        .execute("UPDATE chunks SET embedding = ?2 WHERE id = ?1 AND embedding IS NULL", params![chunk.0, blob])
        .map_err(storage)?;
    Ok(n == 1)
}

/// Reads one chunk's text, whether or not its note is soft-deleted.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `chunk` - id of the chunk to read.
///
/// # Returns
/// The chunk text, or `None` if no chunk has that id.
///
/// # Errors
/// * [`Error::Storage`] - the SQLite query fails.
pub fn text(conn: &Connection, chunk: ChunkId) -> Result<Option<String>> {
    conn.query_row("SELECT text FROM chunks WHERE id = ?1", [chunk.0], |r| r.get(0)).optional().map_err(storage)
}

/// Streams every embedded chunk of a live note that matches `filter` to `visit`, decoding each
/// embedding into one reused buffer so the scan allocates once.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `filter` - optional tag and `updated_at` range (after inclusive, before exclusive).
/// * `visit` - called with the chunk id, its note id and the decoded embedding; the slice is
///   only valid during the call.
///
/// # Errors
/// * [`Error::Storage`] - the SQLite query fails or a stored blob's length is not a multiple of 4.
pub fn for_each_embedding(
    conn: &Connection,
    filter: &SearchFilter,
    visit: &mut dyn FnMut(ChunkId, NoteId, &[f32]),
) -> Result<()> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT c.id, c.note_id, c.embedding FROM chunks c JOIN notes n ON n.id = c.note_id \
             WHERE c.embedding IS NOT NULL AND n.deleted_at IS NULL \
             AND (?1 IS NULL OR EXISTS (SELECT 1 FROM note_tags t WHERE t.note_id = n.id AND t.tag = ?1)) \
             AND (?2 IS NULL OR n.updated_at >= ?2) AND (?3 IS NULL OR n.updated_at < ?3)",
        )
        .map_err(storage)?;
    let tag = filter.tag.as_ref().map(|t| &t.0);
    let mut rows = stmt.query(params![tag, filter.updated_after, filter.updated_before]).map_err(storage)?;
    let mut buf = Vec::new();
    while let Some(r) = rows.next().map_err(storage)? {
        let id = ChunkId(r.get(0).map_err(storage)?);
        let note = NoteId(r.get(1).map_err(storage)?);
        decode(r.get_ref(2).map_err(storage)?, id, &mut buf)?;
        visit(id, note, &buf);
    }
    Ok(())
}

/// Decodes an f32 little-endian embedding blob into `buf`, replacing its previous contents.
///
/// # Arguments
/// * `value` - the raw SQLite column value.
/// * `id` - chunk id, used only in the error message.
/// * `buf` - output buffer, cleared then filled.
///
/// # Errors
/// * [`Error::Storage`] - `value` is not a blob or its length is not a multiple of 4.
pub(crate) fn decode(value: ValueRef<'_>, id: ChunkId, buf: &mut Vec<f32>) -> Result<()> {
    let blob = value.as_blob().map_err(|e| Error::Storage(e.to_string()))?;
    if blob.len() % 4 != 0 {
        return Err(Error::Storage(format!("chunk {} has a malformed embedding", id.0)));
    }
    buf.clear();
    buf.extend(blob.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
    Ok(())
}

//! Keeps a note's chunk rows in step with its body, reusing rows (and embeddings) by text hash.

use std::collections::HashMap;

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use super::chunks::to_i64;
use super::storage;
use crate::domain::{ChunkId, NewChunk, NoteId};
use crate::error::Result;

/// Hashes a chunk's text with SHA-256; the digest is stored as `chunks.text_hash` and lets an
/// edit recognise chunks whose text did not change.
///
/// # Arguments
/// * `text` - the chunk text.
///
/// # Returns
/// The 32-byte digest.
pub(crate) fn text_hash(text: &str) -> Vec<u8> {
    Sha256::digest(text.as_bytes()).to_vec()
}

/// Replaces the chunks of `note` with `chunks`. A stored chunk whose text hash matches a new
/// one is kept, embedding included, with its position updated (embedded duplicates are reused
/// first); the rest are deleted and the new ones inserted without embeddings. Call it inside the
/// write transaction.
///
/// # Arguments
/// * `conn` - the write transaction's connection.
/// * `note` - the note whose chunks are replaced.
/// * `chunks` - the new chunks, with sequence numbers and byte offsets.
///
/// # Returns
/// Ids of deleted chunks that had an embedding, for the vector tier to drop.
///
/// # Errors
/// * [`Error::InvalidInput`](crate::error::Error::InvalidInput) - an offset exceeds `i64::MAX`.
/// * [`Error::Storage`](crate::error::Error::Storage) - a SQLite statement fails.
pub fn replace(conn: &Connection, note: NoteId, chunks: &[NewChunk]) -> Result<Vec<ChunkId>> {
    let mut old: HashMap<Vec<u8>, Vec<(ChunkId, bool)>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare_cached(
                "SELECT id, text_hash, embedding IS NOT NULL FROM chunks WHERE note_id = ?1 ORDER BY embedding IS NOT NULL, id DESC",
            )
            .map_err(storage)?;
        let mut rows = stmt.query([note.0]).map_err(storage)?;
        while let Some(r) = rows.next().map_err(storage)? {
            let hash: Vec<u8> = r.get(1).map_err(storage)?;
            let row = (ChunkId(r.get(0).map_err(storage)?), r.get(2).map_err(storage)?);
            old.entry(hash).or_default().push(row);
        }
    }
    let mut keep =
        conn.prepare_cached("UPDATE chunks SET seq = ?2, start = ?3, end = ?4 WHERE id = ?1").map_err(storage)?;
    let mut insert = conn
        .prepare_cached("INSERT INTO chunks(note_id, seq, start, end, text, text_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
        .map_err(storage)?;
    for c in chunks {
        let hash = text_hash(&c.text);
        let (start, end) = (to_i64(c.start)?, to_i64(c.end)?);
        match old.get_mut(&hash).and_then(Vec::pop) {
            Some((id, _)) => keep.execute(params![id.0, c.seq, start, end]).map_err(storage)?,
            None => insert.execute(params![note.0, c.seq, start, end, c.text, hash]).map_err(storage)?,
        };
    }
    let mut delete = conn.prepare_cached("DELETE FROM chunks WHERE id = ?1").map_err(storage)?;
    let mut gone = Vec::new();
    for (id, embedded) in old.into_values().flatten() {
        delete.execute([id.0]).map_err(storage)?;
        if embedded {
            gone.push(id);
        }
    }
    Ok(gone)
}

/// Schema migration that adds the `chunks.text_hash` column (the fourth entry of the schema
/// migration list, schema version 4) and backfills it for every existing chunk, so later edits
/// keep the embeddings of unchanged chunks. Runs inside the migration transaction.
///
/// # Arguments
/// * `conn` - the connection being migrated.
///
/// # Errors
/// Any `rusqlite` error from the `ALTER TABLE`, the read or an update.
pub(crate) fn add_text_hash(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("ALTER TABLE chunks ADD COLUMN text_hash BLOB NOT NULL DEFAULT x'';")?;
    let mut read = conn.prepare("SELECT id, text FROM chunks")?;
    let mut write = conn.prepare("UPDATE chunks SET text_hash = ?2 WHERE id = ?1")?;
    let mut rows = read.query([])?;
    while let Some(r) = rows.next()? {
        let (id, text): (i64, String) = (r.get(0)?, r.get(1)?);
        write.execute(params![id, text_hash(&text)])?;
    }
    Ok(())
}

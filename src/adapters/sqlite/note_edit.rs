//! Versioned note edits: body, tags and the chunk rows they imply.

use rusqlite::{Connection, params};

use super::notes::{by_id, set_tags};
use super::{chunks, now, storage};
use crate::domain::{ChunkId, NewChunk, Note, NoteId, Tag};
use crate::error::{Error, Result};

/// Loads live note `id` and applies a versioned edit to it via `update`. Call it inside the
/// write transaction so the version check and the writes are atomic.
///
/// # Arguments
/// * `conn` - the write transaction's connection.
/// * `id` - the note to edit.
/// * `expected` - the version the caller last read.
/// * `body` - the new body.
/// * `tags` - the new tag set.
/// * `chunks` - the new body's chunks; used only if the body changed.
///
/// # Returns
/// The edited note, and the ids of embedded chunks the edit deleted.
///
/// # Errors
/// * [`Error::NotFound`] - no live note has this id.
/// * [`Error::VersionMismatch`] - the stored version differs from `expected`.
/// * [`Error::InvalidInput`] - a chunk offset exceeds `i64::MAX`.
/// * [`Error::Storage`] - a SQLite statement fails.
pub fn edit(
    conn: &Connection,
    id: NoteId,
    expected: i64,
    body: &str,
    tags: &[Tag],
    chunks: &[NewChunk],
) -> Result<(Note, Vec<ChunkId>)> {
    let current = by_id(conn, id)?.ok_or(Error::NotFound)?;
    update(conn, current, expected, body, tags, chunks)
}

/// Applies a versioned edit to an already loaded `current` note and builds the result without
/// reading the row back. The version always goes up by one; if the body changed, chunks are
/// replaced (unchanged texts keep their embeddings) and `updated_at` is set to now, while a
/// tags-only edit keeps `updated_at`. Tags are always replaced.
///
/// # Arguments
/// * `conn` - the write transaction's connection.
/// * `current` - the note as stored; consumed and returned updated.
/// * `expected` - the version the caller last read.
/// * `body` - the new body.
/// * `tags` - the new tag set; returned sorted and deduplicated.
/// * `chunks` - the new body's chunks; used only if the body changed.
///
/// # Returns
/// The edited note, and the ids of embedded chunks the edit deleted (empty if the body is unchanged).
///
/// # Errors
/// * [`Error::VersionMismatch`] - `current.version` differs from `expected`.
/// * [`Error::InvalidInput`] - a chunk offset exceeds `i64::MAX`.
/// * [`Error::Storage`] - a SQLite statement fails.
pub(crate) fn update(
    conn: &Connection,
    mut current: Note,
    expected: i64,
    body: &str,
    tags: &[Tag],
    chunks: &[NewChunk],
) -> Result<(Note, Vec<ChunkId>)> {
    if current.version != expected {
        return Err(Error::VersionMismatch { expected, actual: current.version });
    }
    let id = current.id;
    let mut gone = Vec::new();
    if current.body == body {
        conn.execute("UPDATE notes SET version = version + 1 WHERE id = ?1", params![id.0]).map_err(storage)?;
    } else {
        let t = now();
        conn.execute(
            "UPDATE notes SET body = ?2, updated_at = ?3, version = version + 1 WHERE id = ?1",
            params![id.0, body, t],
        )
        .map_err(storage)?;
        gone = chunks::replace(conn, id, chunks)?;
        body.clone_into(&mut current.body);
        current.updated_at = t;
    }
    set_tags(conn, id, tags)?;
    current.version += 1;
    current.tags = tags.to_vec();
    current.tags.sort();
    current.tags.dedup();
    Ok((current, gone))
}

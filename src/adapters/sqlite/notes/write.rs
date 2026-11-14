//! Note creation (with optional overwrite by title) and hard deletion.

use rusqlite::{Connection, params};

use super::super::{chunks, note_edit, now, storage};
use super::{by_id, by_title, set_tags};
use crate::domain::{ChunkId, NewChunk, NewNote, Note, NoteId};
use crate::error::{Error, Result};

/// Inserts a note with its tags and chunks, or, when `overwrite` is set and a live note with the
/// same title (case-insensitive) exists, rewrites that note in place: the title takes the new
/// casing, and the body, tags and chunks are replaced through the regular edit path (bumping its
/// version). Runs several statements; call inside a transaction so they commit atomically.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `note` - title, body and tags to store.
/// * `chunks` - the body split into chunks, stored without embeddings.
/// * `overwrite` - replace a live note with the same title instead of failing.
///
/// # Returns
/// The stored note, and the ids of embedded chunks the overwrite deleted (empty for an insert).
///
/// # Errors
/// * [`Error::Conflict`] - a live note has this title and `overwrite` is false.
/// * [`Error::NotFound`] - the inserted note cannot be read back.
/// * [`Error::Storage`] - a SQLite statement fails.
pub fn create(conn: &Connection, note: &NewNote, chunks: &[NewChunk], overwrite: bool) -> Result<(Note, Vec<ChunkId>)> {
    if let Some(existing) = by_title(conn, &note.title)? {
        if !overwrite {
            return Err(Error::Conflict { existing_id: existing.id });
        }
        conn.execute("UPDATE notes SET title = ?2 WHERE id = ?1", params![existing.id.0, note.title])
            .map_err(storage)?;
        let version = existing.version;
        let existing = Note { title: note.title.clone(), ..existing };
        return note_edit::update(conn, existing, version, &note.body, &note.tags, chunks);
    }
    let t = now();
    conn.execute(
        "INSERT INTO notes(title, body, created_at, updated_at, version) VALUES (?1, ?2, ?3, ?3, 1)",
        params![note.title, note.body, t],
    )
    .map_err(storage)?;
    let id = NoteId(conn.last_insert_rowid());
    set_tags(conn, id, &note.tags)?;
    chunks::replace(conn, id, chunks)?;
    Ok((by_id(conn, id)?.ok_or(Error::NotFound)?, Vec::new()))
}

/// Deletes a live note row outright: the FTS trigger drops its index entry and foreign keys
/// cascade-delete its tags and chunks, so no text survives. Ids are `AUTOINCREMENT` and never reused.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `id` - the note to delete.
///
/// # Errors
/// * [`Error::NotFound`] - no live note has this id.
/// * [`Error::Storage`] - the SQLite delete fails.
pub fn delete(conn: &Connection, id: NoteId) -> Result<()> {
    let n = conn.execute("DELETE FROM notes WHERE id = ?1 AND deleted_at IS NULL", [id.0]).map_err(storage)?;
    if n == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

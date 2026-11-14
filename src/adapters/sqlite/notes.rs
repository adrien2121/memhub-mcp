//! Note rows and tags: the `NoteStore` CRUD half.

use rusqlite::{Connection, OptionalExtension, Row, params};

pub use super::note_edit::edit;
use super::storage;
use crate::domain::{Note, NoteId, Tag};
use crate::error::Result;

mod write;

pub use write::{create, delete};

/// Columns selected by [`from_row`].
pub(crate) const COLUMNS: &str = "id, title, body, created_at, updated_at, version";

/// Builds a note from a row selected with [`COLUMNS`]; tags are left empty for the caller to fill.
///
/// # Arguments
/// * `r` - a result row whose columns are exactly [`COLUMNS`], in order.
///
/// # Returns
/// The note with an empty `tags` vector.
///
/// # Errors
/// Any [`rusqlite::Error`] from reading or converting a column.
pub(crate) fn from_row(r: &Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: NoteId(r.get(0)?),
        title: r.get(1)?,
        body: r.get(2)?,
        tags: Vec::new(),
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
        version: r.get(5)?,
    })
}

/// Loads the tags of `note` from `note_tags`, sorted, replacing `note.tags`.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `note` - the note whose `tags` are overwritten in place.
///
/// # Errors
/// * [`crate::error::Error::Storage`] - the SQLite query fails.
pub(crate) fn attach_tags(conn: &Connection, note: &mut Note) -> Result<()> {
    let mut stmt = conn.prepare_cached("SELECT tag FROM note_tags WHERE note_id = ?1 ORDER BY tag").map_err(storage)?;
    note.tags = stmt
        .query_map([note.id.0], |r| r.get(0).map(Tag))
        .map_err(storage)?
        .collect::<rusqlite::Result<_>>()
        .map_err(storage)?;
    Ok(())
}

/// Reads the live (not soft-deleted) note matching a one-parameter SQL condition, with its tags.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `filter` - trusted SQL condition using `?1`, e.g. `id = ?1` or `title = ?1`.
/// * `key` - value bound to `?1`.
///
/// # Returns
/// The note, or `None` if no live note matches.
///
/// # Errors
/// * [`crate::error::Error::Storage`] - a SQLite query fails.
fn load_where(conn: &Connection, filter: &str, key: &dyn rusqlite::ToSql) -> Result<Option<Note>> {
    let sql = format!("SELECT {COLUMNS} FROM notes WHERE {filter} AND deleted_at IS NULL");
    let note = conn.prepare_cached(&sql).and_then(|mut s| s.query_row([key], from_row).optional()).map_err(storage)?;
    match note {
        Some(mut n) => attach_tags(conn, &mut n).map(|()| Some(n)),
        None => Ok(None),
    }
}

/// Reads a live note by id, with its tags.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `id` - the note id.
///
/// # Returns
/// The note, or `None` if it does not exist or is soft-deleted.
///
/// # Errors
/// * [`crate::error::Error::Storage`] - a SQLite query fails.
pub fn by_id(conn: &Connection, id: NoteId) -> Result<Option<Note>> {
    load_where(conn, "id = ?1", &id.0)
}

/// Reads a live note by title, case-insensitively (the `title` column is `COLLATE NOCASE`).
///
/// # Arguments
/// * `conn` - open database connection.
/// * `title` - the title to look up.
///
/// # Returns
/// The note with its tags, or `None` if no live note has this title.
///
/// # Errors
/// * [`crate::error::Error::Storage`] - a SQLite query fails.
pub fn by_title(conn: &Connection, title: &str) -> Result<Option<Note>> {
    load_where(conn, "title = ?1", &title)
}

/// Replaces the tags of note `id` with `tags`; duplicates are stored once. Call inside a
/// transaction: outside one, SQLite commits the delete and each insert separately.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `id` - the note whose tags are replaced.
/// * `tags` - the complete new tag set.
///
/// # Errors
/// * [`crate::error::Error::Storage`] - a SQLite statement fails.
pub(crate) fn set_tags(conn: &Connection, id: NoteId, tags: &[Tag]) -> Result<()> {
    conn.execute("DELETE FROM note_tags WHERE note_id = ?1", [id.0]).map_err(storage)?;
    let mut stmt =
        conn.prepare_cached("INSERT OR IGNORE INTO note_tags(note_id, tag) VALUES (?1, ?2)").map_err(storage)?;
    for t in tags {
        stmt.execute(params![id.0, t.0]).map_err(storage)?;
    }
    Ok(())
}

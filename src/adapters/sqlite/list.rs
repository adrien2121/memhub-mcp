//! Keyset-paginated note listing on `(updated_at, id)`, newest first.

use rusqlite::{Connection, params};

use super::notes::{COLUMNS, from_row};
use super::storage;
use crate::domain::{Cursor, ListQuery, Note, Page, Tag};
use crate::error::{Error, Result};

/// Encodes the keyset position of `n` as an opaque `"<updated_at>.<id>"` page cursor.
///
/// # Arguments
/// * `n` - the last note of the returned page.
///
/// # Returns
/// The cursor that resumes listing just after `n`.
fn encode(n: &Note) -> Cursor {
    Cursor(format!("{}.{}", n.updated_at, n.id.0))
}

/// Parses a `"<updated_at>.<id>"` cursor produced by [`encode`].
///
/// # Arguments
/// * `c` - the cursor supplied by the client.
///
/// # Returns
/// `(updated_at, note id)` of the last note on the previous page.
///
/// # Errors
/// * [`Error::InvalidInput`] - no `.` separator, or either part is not an `i64`.
fn decode(c: &Cursor) -> Result<(i64, i64)> {
    let bad = || Error::InvalidInput("bad cursor".into());
    let (t, id) = c.0.split_once('.').ok_or_else(bad)?;
    Ok((t.parse().map_err(|_| bad())?, id.parse().map_err(|_| bad())?))
}

/// Largest page size; larger requests are capped to it.
pub const MAX_LIMIT: usize = 100;

/// Lists one page of live (not soft-deleted) notes, newest `updated_at` first (ties by id
/// descending), with their tags; read-only. Fetches one extra row to know whether more remain.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `q` - optional tag, `updated_at` range (after inclusive, before exclusive), cursor from a
///   previous page, and page size; a `limit` above [`MAX_LIMIT`] is capped to it.
///
/// # Returns
/// The page of notes and, if more notes follow, the cursor of the next page.
///
/// # Errors
/// * [`Error::InvalidInput`] - the cursor is malformed or `limit` is zero.
/// * [`Error::Storage`] - a SQLite query fails.
pub fn list(conn: &Connection, q: &ListQuery) -> Result<Page<Note>> {
    let after = q.cursor.as_ref().map(decode).transpose()?;
    if q.limit == 0 {
        return Err(Error::InvalidInput("limit must be between 1 and 100".into()));
    }
    let page = q.limit.min(MAX_LIMIT);
    let sql = format!(
        "SELECT {COLUMNS} FROM notes n WHERE deleted_at IS NULL \
         AND (?1 IS NULL OR EXISTS (SELECT 1 FROM note_tags t WHERE t.note_id = n.id AND t.tag = ?1)) \
         AND (?2 IS NULL OR updated_at >= ?2) AND (?3 IS NULL OR updated_at < ?3) \
         AND (?4 IS NULL OR (updated_at, id) < (?4, ?5)) \
         ORDER BY updated_at DESC, id DESC LIMIT ?6"
    );
    let mut stmt = conn.prepare_cached(&sql).map_err(storage)?;
    let mut items: Vec<Note> = stmt
        .query_map(
            params![
                q.tag.as_ref().map(|t| &t.0),
                q.updated_after,
                q.updated_before,
                after.map(|a| a.0),
                after.map(|a| a.1),
                i64::try_from(page + 1).unwrap_or(i64::MAX)
            ],
            from_row,
        )
        .map_err(storage)?
        .collect::<rusqlite::Result<_>>()
        .map_err(storage)?;
    let next_cursor = if items.len() > page {
        items.truncate(page);
        items.last().map(encode)
    } else {
        None
    };
    attach_all_tags(conn, &mut items)?;
    Ok(Page { items, next_cursor })
}

/// Fills the tags (sorted) of every note in `items` with one query over all their ids.
///
/// # Arguments
/// * `conn` - open database connection.
/// * `items` - notes whose empty `tags` vectors get filled in place.
///
/// # Errors
/// * [`Error::Storage`] - the SQLite query fails.
fn attach_all_tags(conn: &Connection, items: &mut [Note]) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let ids = format!("[{}]", items.iter().map(|n| n.id.0.to_string()).collect::<Vec<_>>().join(","));
    let mut stmt = conn
        .prepare_cached(
            "SELECT note_id, tag FROM note_tags WHERE note_id IN (SELECT value FROM json_each(?1)) ORDER BY tag",
        )
        .map_err(storage)?;
    let rows = stmt.query_map([ids], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).map_err(storage)?;
    for row in rows {
        let (id, tag) = row.map_err(storage)?;
        if let Some(n) = items.iter_mut().find(|n| n.id.0 == id) {
            n.tags.push(Tag(tag));
        }
    }
    Ok(())
}

//! FTS5 snippets, with a bounded window for large note bodies.

use rusqlite::{Connection, OptionalExtension, params};

use super::super::storage;
use crate::domain::NoteId;
use crate::error::Result;

/// Bodies longer than this many bytes get their snippet from a window around the first match.
const WINDOW: usize = 8192;

/// Builds the FTS5 `snippet()` of one note for `query`, with matches wrapped in `[`…`]`.
///
/// Bodies over `WINDOW` (8192) bytes are cut to a window around the first literal occurrence of a
/// query term (else their start) and re-indexed in a temporary FTS5 table, so snippet indexing cost
/// stays bounded (locating the window is still a linear scan of the body). Falls back to a snippet
/// of the full indexed note if the window has no match.
///
/// # Arguments
/// * `conn` - open database connection; may create and drop the temp table `temp.snip`.
/// * `query` - prebuilt FTS5 match expression.
/// * `note` - id of the note to excerpt.
///
/// # Returns
/// The snippet, or an empty string if the note does not match `query` (or does not exist).
///
/// # Errors
/// * [`crate::error::Error::Storage`] - a SQLite statement fails, including FTS5 syntax errors.
pub fn snippet(conn: &Connection, query: &str, note: NoteId) -> Result<String> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT n.title, n.body, length(CAST(n.body AS BLOB)) > ?3 FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid \
             WHERE notes_fts MATCH ?1 AND notes_fts.rowid = ?2",
        )
        .map_err(storage)?;
    let row: Option<(String, String, bool)> = stmt
        .query_row(params![query, note.0, WINDOW as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()
        .map_err(storage)?;
    let Some((title, body, large)) = row else { return Ok(String::new()) };
    if large && let Some(s) = windowed(conn, query, &title, window(&body, query))? {
        return Ok(s);
    }
    let mut stmt = conn
        .prepare_cached(
            "SELECT snippet(notes_fts, -1, '[', ']', '…', 16) FROM notes_fts \
             WHERE notes_fts MATCH ?1 AND notes_fts.rowid = ?2",
        )
        .map_err(storage)?;
    stmt.query_row(params![query, note.0], |r| r.get(0)).map_err(storage)
}

/// Snippets `title` + `body` alone by indexing them in a temporary FTS5 table `temp.snip`, which
/// is created and dropped by this call. It uses the tokenizer declared in the `notes_fts` schema
/// (configurable, so read from `sqlite_master`; `unicode61` if none is declared).
///
/// # Arguments
/// * `conn` - open database connection.
/// * `query` - prebuilt FTS5 match expression.
/// * `title` - the note title.
/// * `body` - the body window to index.
///
/// # Returns
/// The snippet, or `None` if the window does not match, e.g. the hit was stemmed-only or outside it.
///
/// # Errors
/// * [`crate::error::Error::Storage`] - a SQLite statement fails.
fn windowed(conn: &Connection, query: &str, title: &str, body: &str) -> Result<Option<String>> {
    let tokenizer: String =
        conn.query_row("SELECT sql FROM sqlite_master WHERE name = 'notes_fts'", [], |r| r.get(0)).map_err(storage)?;
    let tokenizer = tokenizer.split("tokenize='").nth(1).and_then(|t| t.split('\'').next()).unwrap_or("unicode61");
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS temp.snip; CREATE VIRTUAL TABLE temp.snip USING fts5(title, body, tokenize='{tokenizer}');"
    ))
    .map_err(storage)?;
    conn.execute("INSERT INTO temp.snip(title, body) VALUES (?1, ?2)", params![title, body]).map_err(storage)?;
    let s: Option<String> = conn
        .query_row("SELECT snippet(snip, -1, '[', ']', '…', 16) FROM temp.snip WHERE snip MATCH ?1", [query], |r| {
            r.get(0)
        })
        .optional()
        .map_err(storage)?;
    conn.execute_batch("DROP TABLE temp.snip").map_err(storage)?;
    Ok(s)
}

/// Picks at most [`WINDOW`] (8192) bytes of `body`, starting half a window before the earliest
/// ASCII-case-insensitive occurrence of any ` OR `-separated term of `query` (quotes stripped).
///
/// # Arguments
/// * `body` - the full note body.
/// * `query` - prebuilt FTS5 match expression of quoted terms joined by ` OR `.
///
/// # Returns
/// A sub-slice of `body` cut on char boundaries; starts at byte 0 if no term occurs literally.
fn window<'a>(body: &'a str, query: &str) -> &'a str {
    let lower = body.to_ascii_lowercase();
    let hit = query
        .split(" OR ")
        .map(|t| t.trim_matches('"').replace("\"\"", "\"").to_ascii_lowercase())
        .filter_map(|t| lower.find(&t))
        .min()
        .unwrap_or(0);
    let floor = |mut i: usize| {
        while !body.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let start = floor(hit.saturating_sub(WINDOW / 2));
    let end = floor((start + WINDOW).min(body.len()));
    body.get(start..end).unwrap_or_default()
}

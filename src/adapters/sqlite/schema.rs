//! SQLite schema, pragmas and forward-only migrations.

use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior};

use super::migrations::{MIGRATIONS, Step};

/// Largest size, in bytes, the WAL file keeps after a checkpoint; without it SQLite reuses the
/// file at its peak size forever.
const WAL_SIZE_LIMIT: i64 = 1024 * 1024;

/// Applies per-connection settings: a 5-second busy timeout, WAL journal mode, a 1 MiB WAL size
/// limit after checkpoints, and foreign-key enforcement. Every new connection needs them, write
/// and read alike.
///
/// # Arguments
/// * `conn` - a freshly opened connection.
///
/// # Errors
/// Any rusqlite error from setting a pragma.
pub fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))?;
    conn.query_row(&format!("PRAGMA journal_size_limit={WAL_SIZE_LIMIT}"), [], |_| Ok(()))?;
    conn.pragma_update(None, "foreign_keys", "ON")
}

/// Copies every WAL page into the main file and truncates the WAL to zero bytes.
///
/// # Arguments
/// * `path` - database file.
///
/// # Errors
/// [`crate::error::Error::Storage`] - the file cannot be opened, or readers kept the checkpoint
/// from completing within the busy timeout.
pub fn checkpoint_truncate(path: &std::path::Path) -> crate::error::Result<()> {
    let conn = super::connect(path)?;
    let busy: i64 = conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
        .map_err(|e| crate::error::Error::Storage(e.to_string()))?;
    if busy != 0 {
        return Err(crate::error::Error::Storage("WAL checkpoint blocked by active readers".into()));
    }
    Ok(())
}

/// Checks that the linked SQLite build has the FTS5 full-text extension by creating and dropping
/// a temporary FTS5 table, so a missing extension fails at startup instead of on first search.
///
/// # Arguments
/// * `conn` - any connection.
///
/// # Errors
/// Any rusqlite error, notably "no such module: fts5" when the extension is absent.
pub fn check_fts5(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE VIRTUAL TABLE temp.fts5_probe USING fts5(x); DROP TABLE temp.fts5_probe;")
}

/// Brings the schema up to date. Applies every migration whose index is at or above the stored
/// `PRAGMA user_version`, then sets `user_version` to the migration count. Everything runs in
/// one `IMMEDIATE` transaction, so a failure leaves the database unchanged.
///
/// # Arguments
/// * `conn` - the write connection.
///
/// # Errors
/// * [`Error::Storage`](crate::error::Error::Storage) - `user_version` is higher than this
///   binary's migration count (a newer binary wrote the database), or any statement fails.
pub fn migrate(conn: &mut Connection) -> crate::error::Result<()> {
    let storage = |e: rusqlite::Error| crate::error::Error::Storage(e.to_string());
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
    let current: u32 = tx.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(storage)?;
    let target = u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX);
    if current > target {
        return Err(crate::error::Error::Storage(format!(
            "database schema version {current} is newer than this binary supports ({target}); upgrade memhub-mcp"
        )));
    }
    for step in MIGRATIONS.iter().skip(current as usize) {
        match step {
            Step::Sql(sql) => tx.execute_batch(sql),
            Step::Rust(run) => run(&tx),
        }
        .map_err(storage)?;
    }
    if current < target {
        tx.pragma_update(None, "user_version", target).map_err(storage)?;
    }
    tx.commit().map_err(storage)
}

/// Tokenizer of the `notes_fts` table created by the initial schema migration (schema version 1);
/// assumed when `meta` has no `fts_tokenizer` record.
const MIGRATED_TOKENIZER: &str = "porter unicode61";

/// Drops and rebuilds the `notes_fts` full-text index from `notes` when `tokenizer` differs from
/// the one in `meta` (or, with no record, the initial schema's `porter unicode61`). Records
/// `tokenizer` in `meta`. Runs in one `IMMEDIATE` transaction.
///
/// # Arguments
/// * `conn` - the write connection.
/// * `tokenizer` - FTS5 tokenizer spec spliced into SQL verbatim; callers must pass a validated value.
///
/// # Errors
/// Any rusqlite error; the transaction rolls back and the old index stays.
pub fn sync_tokenizer(conn: &mut Connection, tokenizer: &str) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored: Option<String> = tx
        .query_row("SELECT value FROM meta WHERE key = 'fts_tokenizer'", [], |r| r.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?;
    if stored.as_deref().unwrap_or(MIGRATED_TOKENIZER) != tokenizer {
        tx.execute_batch(&format!(
            "DROP TABLE notes_fts;
             CREATE VIRTUAL TABLE notes_fts USING fts5(title, body, content='notes',
               content_rowid='id', tokenize='{tokenizer}');
             INSERT INTO notes_fts(notes_fts) VALUES ('rebuild');"
        ))?;
    }
    if stored.as_deref() != Some(tokenizer) {
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('fts_tokenizer', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [tokenizer],
        )?;
    }
    tx.commit()
}

#[cfg(test)]
mod tests;

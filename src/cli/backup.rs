//! `memhub-mcp backup <path>`: copies the database with `VACUUM INTO`.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::config::Config;
use crate::error::{Error, Result};

/// Writes a consistent, compacted copy of the SQLite database at `db` to the new file `dest`
/// using `VACUUM INTO`. Safe while the daemon runs: it reads one snapshot, waiting up to 5 s
/// for a busy database.
///
/// # Arguments
/// * `db` - Path of the live database.
/// * `dest` - Path of the backup file to create; must not exist and must be UTF-8.
///
/// # Returns
/// `Ok(())` once the backup file is complete.
///
/// # Errors
/// * [`Error::InvalidInput`] - `dest` already exists or is not valid UTF-8.
/// * [`Error::Storage`] - `db` does not exist, or SQLite cannot open it, set the busy timeout, or write the copy.
pub fn backup(db: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        return Err(Error::InvalidInput(format!("{} already exists", dest.display())));
    }
    let dest_str = dest.to_str().ok_or_else(|| Error::InvalidInput("backup path is not UTF-8".into()))?;
    let storage = |e: rusqlite::Error| Error::Storage(e.to_string());
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
    let conn = Connection::open_with_flags(db, flags).map_err(storage)?;
    conn.busy_timeout(Duration::from_millis(5000)).map_err(storage)?;
    conn.execute("VACUUM INTO ?1", [dest_str]).map_err(storage)?;
    Ok(())
}

/// Runs `memhub-mcp backup <dest>`: backs up the configured database to `dest` and prints the
/// destination on stdout.
///
/// # Arguments
/// * `config` - Supplies `db_path`, the database to copy.
/// * `dest` - Path of the backup file to create.
///
/// # Returns
/// `Ok(())` once the backup is written.
///
/// # Errors
/// * [`Error::InvalidInput`] - `dest` already exists or is not valid UTF-8.
/// * [`Error::Storage`] - SQLite failed to read the database or write the copy.
pub fn run(config: &Config, dest: &Path) -> Result<()> {
    backup(&config.db_path, dest)?;
    println!("backup written to {}", dest.display());
    Ok(())
}

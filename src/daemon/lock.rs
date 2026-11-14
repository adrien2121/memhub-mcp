//! Exclusive database lock file shared by the daemon and the writing CLI jobs.

use std::fs::File;
use std::io::ErrorKind;

use crate::config::Config;
use crate::error::{Error, Result};

/// Creates the lock file next to the database (creating its directory if needed) and takes an
/// exclusive, non-blocking OS lock on it; the OS releases the lock when the returned file drops
/// or the process dies.
///
/// The daemon holds it for its lifetime, and the `import` (bulk-load basic-memory Markdown) and
/// `reembed` (re-embed every note) CLI subcommands hold it for theirs, so no CLI job writes
/// vectors that the daemon's in-memory vector index would miss.
///
/// # Arguments
/// * `config` - Supplies the lock file path ([`Config::lock_path`]) and the database path used in
///   the error message.
///
/// # Returns
/// The open lock file; keep it alive for as long as the lock must be held.
///
/// # Errors
/// * [`Error::Io`] - Kind `ResourceBusy` if another process already holds the lock; otherwise the
///   directory or file can't be created or the lock call fails.
pub fn lock(config: &Config) -> Result<File> {
    let path = config.lock_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = File::create(&path)?;
    match fs4::FileExt::try_lock(&file) {
        Ok(()) => Ok(file),
        Err(fs4::TryLockError::WouldBlock) => Err(Error::Io(std::io::Error::new(
            ErrorKind::ResourceBusy,
            format!(
                "{} is in use by a running daemon, import or reembed (lock {}); stop it and retry",
                config.db_path.display(),
                path.display()
            ),
        ))),
        Err(fs4::TryLockError::Error(e)) => Err(Error::Io(e)),
    }
}

//! `memhub-mcp import <dir>`: copies basic-memory Markdown into the database.

use std::path::Path;
use std::sync::Arc;

use crate::adapters::markdown;
use crate::app::embed_worker;
use crate::app::notes::NoteService;
use crate::config::Config;
use crate::daemon;
use crate::error::{Error, Result};
use crate::wiring;

/// Outcome counts of one import.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Notes written.
    pub imported: usize,
    /// Files skipped because their title already exists or they exceed the body size limit.
    pub skipped: usize,
    /// Files that couldn't be read or written.
    pub failed: usize,
}

/// Imports every `.md` file under `dir` (basic-memory layout) through `service`, never
/// overwriting existing notes. Per-file problems are printed on stderr and counted, not returned.
///
/// # Arguments
/// * `service` - Note service that validates and stores each note.
/// * `dir` - Root directory to walk recursively.
///
/// # Returns
/// A [`Report`]: files imported; files skipped because their title is taken or the body is too
/// large; and files that could not be read, parsed or stored.
///
/// # Errors
/// * [`Error::Io`] - `dir` itself cannot be read; unreadable subdirectories count as failed.
pub fn import(service: &NoteService, dir: &Path) -> Result<Report> {
    let walk = markdown::walk(dir)?;
    let mut report = Report { failed: walk.failed, ..Report::default() };
    for path in walk.files {
        let rel = path.strip_prefix(dir).unwrap_or(&path);
        let outcome = markdown::read(dir, rel).and_then(|src| service.write(src.note, false));
        match outcome {
            Ok(_) => report.imported += 1,
            Err(Error::Conflict { existing_id }) => {
                eprintln!("skipped {}: title already used by note {}", rel.display(), existing_id.0);
                report.skipped += 1;
            }
            Err(e @ Error::TooLarge(_)) => {
                eprintln!("skipped {}: {e}", rel.display());
                report.skipped += 1;
            }
            Err(e) => {
                eprintln!("failed {}: {e}", rel.display());
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// Runs `memhub-mcp import <dir>`: imports `dir` into the configured database, then embeds the
/// new chunks with progress on stderr and prints the counts on stdout. If the embedder can't be
/// loaded, notes are imported unembedded (a warning says to run `reembed`). Holds the database
/// lock file throughout, so it never runs beside the daemon, whose in-memory vector index would
/// miss these writes.
///
/// # Arguments
/// * `config` - Supplies the database, lock and embedder paths.
/// * `dir` - Root directory of the Markdown notes.
///
/// # Returns
/// `Ok(())` when no file failed.
///
/// # Errors
/// * [`Error::Io`] - The lock is held (kind `ResourceBusy`) or `dir` cannot be walked.
/// * [`Error::Storage`] - The database cannot be opened or written.
/// * [`Error::Embedder`] - Embedding the imported chunks fails.
/// * [`Error::InvalidInput`] - At least one file failed to import.
pub fn run(config: &Config, dir: &Path) -> Result<()> {
    let _lock = daemon::lock(config)?;
    let store = wiring::store(config)?;
    let embedder = match wiring::embedder(config) {
        Ok(e) => {
            embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
            Some(e)
        }
        Err(e) => {
            eprintln!("embedder unavailable, notes stay unembedded until `reembed`: {e}");
            None
        }
    };
    let service = NoteService::new(Arc::clone(&store), embedder.clone(), None);
    let report = import(&service, dir)?;
    if let Some(e) = &embedder {
        let done = embed_worker::drain(store.as_ref(), e, &mut |n| eprint!("\rembedding: {n} chunks"))?;
        eprintln!("\rembedding: {done} chunks embedded");
    }
    println!("imported {}, skipped {}, failed {}", report.imported, report.skipped, report.failed);
    if report.failed > 0 {
        return Err(Error::InvalidInput(format!("{} files failed to import", report.failed)));
    }
    Ok(())
}

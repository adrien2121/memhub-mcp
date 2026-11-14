//! `memhub-mcp reembed`: rebuilds every vector with the configured embedder.

use crate::app::embed_worker;
use crate::config::Config;
use crate::daemon;
#[cfg(doc)]
use crate::error::Error;
use crate::error::Result;
use crate::wiring;

/// Runs `memhub-mcp reembed`: re-chunks and re-embeds every note with the configured embedder,
/// updating a `done/total` progress line on stderr. Holds the database lock file throughout, so
/// it never runs beside the daemon.
///
/// # Arguments
/// * `config` - Supplies the database, lock and embedder (model and onnxruntime) paths.
///
/// # Returns
/// `Ok(())` once every chunk has a new vector.
///
/// # Errors
/// * [`Error::Io`] - The lock is held by the daemon or another job (kind `ResourceBusy`), or
///   the lock file cannot be opened.
/// * [`Error::Embedder`] - The model cannot be loaded or inference fails.
/// * [`Error::Storage`] - The database cannot be opened, read or written.
pub fn run(config: &Config) -> Result<()> {
    let _lock = daemon::lock(config)?;
    let store = wiring::store(config)?;
    let embedder = wiring::embedder(config)?;
    let done = embed_worker::reembed(store.as_ref(), &embedder, &mut |done, total| {
        eprint!("\rreembed: {done}/{total} chunks");
    })?;
    eprintln!("\rreembed: {done} chunks embedded");
    Ok(())
}

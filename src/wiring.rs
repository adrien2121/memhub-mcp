//! Builds the concrete adapters and hands them to the app services.

use std::sync::{Arc, Mutex};

use crate::adapters::ort::OrtEmbedder;
use crate::adapters::sqlite::SqliteStore;
use crate::app::embed_worker::{self, Drainer};
use crate::app::notes::NoteService;
use crate::app::reembed::{self, ReembedService};
use crate::app::search::SearchService;
use crate::config::Config;
use crate::error::Result;
use crate::mcp::server::MemoryServer;
use crate::ports::embedder::SharedEmbedder;
use crate::ports::store::NoteStore;

/// Opens the SQLite note store at the configured path, creating its parent directory if needed,
/// with the configured full-text tokenizer and exact-search chunk limit.
///
/// # Arguments
/// * `config` - Supplies `db_path`, `fts_tokenizer` and `exact_max_chunks`.
///
/// # Returns
/// The store behind the [`NoteStore`] port, shareable across services.
///
/// # Errors
/// * [`crate::error::Error::Io`] - The database directory can't be created.
/// * [`crate::error::Error::Storage`] - The database can't be opened, migrated or configured.
pub fn store(config: &Config) -> Result<Arc<dyn NoteStore>> {
    if let Some(dir) = config.db_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let store = SqliteStore::open_with_tokenizer(&config.db_path, config.fts_tokenizer.as_sql())?;
    Ok(Arc::new(store.with_exact_max_chunks(config.exact_max_chunks)?))
}

/// Loads the configured ONNX embedding model and tokenizer, wrapped for sharing across threads.
///
/// # Arguments
/// * `config` - Supplies the model, tokenizer and runtime locations.
///
/// # Returns
/// The embedder behind a mutex-guarded [`SharedEmbedder`].
///
/// # Errors
/// * [`crate::error::Error::Embedder`] - The model, tokenizer or ONNX runtime can't be loaded.
pub fn embedder(config: &Config) -> Result<SharedEmbedder> {
    Ok(Arc::new(Mutex::new(Box::new(OrtEmbedder::load(config)?))))
}

/// Builds the MCP server shared by every session and starts embedding pending chunks on a
/// background thread.
///
/// A missing embedder is logged as a warning and leaves search keyword-only. With an embedder, it
/// first lets the store adopt the embedder's identity if no vectors exist yet, and warns when the
/// stored vectors come from a different model (vector search stays off until the `reembed` CLI
/// subcommand re-chunks and re-embeds every note with the configured model).
///
/// # Arguments
/// * `config` - Loaded configuration for the store and embedder.
///
/// # Returns
/// The server and the background drainer, if an embedder loaded; drop the drainer to stop and
/// join its thread.
///
/// # Errors
/// * [`crate::error::Error::Io`] / [`crate::error::Error::Storage`] - As reported by [`store`], or
///   a store failure while checking the embedder identity of stored vectors.
pub fn build(config: &Config) -> Result<(MemoryServer, Option<Drainer>)> {
    let store = store(config)?;
    let embedder = match embedder(config) {
        Ok(e) => Some(e),
        Err(e) => {
            tracing::warn!("embedder unavailable, search is keyword-only: {e}");
            None
        }
    };
    let mut drainer = None;
    if let Some(e) = &embedder {
        embed_worker::adopt_if_fresh(store.as_ref(), e)?;
        if let Some(c) = reembed::change(store.as_ref(), e)? {
            tracing::warn!(
                "stored vectors come from {} but {} is configured; vector search is off until `reembed`",
                c.stored,
                c.configured
            );
        }
        drainer = Some(embed_worker::spawn_drainer(Arc::clone(&store), Arc::clone(e)));
    }
    let wake = drainer.as_ref().map(Drainer::wake);
    let server = MemoryServer::new(
        Arc::new(NoteService::new(Arc::clone(&store), embedder.clone(), wake.clone())),
        Arc::new(SearchService::new(Arc::clone(&store), embedder.clone())),
        Arc::new(ReembedService::new(store, embedder, wake)),
    );
    Ok((server, drainer))
}

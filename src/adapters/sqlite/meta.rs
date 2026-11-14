//! Embedder identity recorded in `meta`, and the reset that goes with changing it.

use rusqlite::{Connection, OptionalExtension};

use super::storage;
use crate::error::Result;

/// Reads the embedding-model identity that the stored vectors were made with, from the `meta`
/// table key `model_identity`.
///
/// # Arguments
/// * `conn` - any connection to the store.
///
/// # Returns
/// The recorded identity, or `None` if no identity was ever recorded.
///
/// # Errors
/// * [`Error::Storage`](crate::error::Error::Storage) - the query fails.
pub fn model_identity(conn: &Connection) -> Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = 'model_identity'", [], |r| r.get(0)).optional().map_err(storage)
}

/// Clears every chunk embedding and failure counter, then records `identity` as the model in
/// `meta`. Call it inside the write transaction so both changes land together.
///
/// # Arguments
/// * `conn` - the write transaction's connection.
/// * `identity` - the new embedding-model identity.
///
/// # Errors
/// * [`Error::Storage`](crate::error::Error::Storage) - either statement fails.
pub fn reset_embeddings(conn: &Connection, identity: &str) -> Result<()> {
    conn.execute(
        "UPDATE chunks SET embedding = NULL, embed_attempts = 0 WHERE embedding IS NOT NULL OR embed_attempts > 0",
        [],
    )
    .map_err(storage)?;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES ('model_identity', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [identity],
    )
    .map_err(storage)?;
    Ok(())
}

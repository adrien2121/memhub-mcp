//! Embeds pending chunks outside the DB write lock; rebuilds every vector on `reembed`.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::app::drainer::charge;
pub use crate::app::drainer::{Drainer, Wake, spawn as spawn_drainer};
pub use crate::app::embedder_lock::{guarded, lock};
use crate::app::embedding;
pub use crate::app::requeue::{chunk_missing, reembed, requeue};
use crate::domain::ChunkId;
use crate::error::Result;
use crate::ports::embedder::{Kind, SharedEmbedder};
use crate::ports::store::NoteStore;

/// Chunks fetched per round trip.
const BATCH: usize = 64;
/// Consecutive failures after which the embedder is probed with [`PROBE_TEXT`]: if the probe
/// fails too it is an outage, so the pass stops and charges none of them; otherwise the chunks
/// themselves are bad, so they are charged and the pass moves on.
const OUTAGE_STREAK: usize = 3;
/// Plain text any working embedder can embed.
const PROBE_TEXT: &str = "probe";

/// Checks whether the stored embeddings were made by `embedder`, by comparing the identity
/// fingerprint recorded in the store with the embedder's own. Briefly locks the embedder.
///
/// # Arguments
/// * `store` - Store holding the recorded model identity.
/// * `embedder` - The configured embedder.
///
/// # Returns
/// `true` when the identities are equal; `false` when they differ or none is recorded.
///
/// # Errors
/// * [`Error::Storage`](crate::error::Error::Storage) - Reading the identity fails.
pub fn identity_matches(store: &dyn NoteStore, embedder: &SharedEmbedder) -> Result<bool> {
    let identity = lock(embedder).identity();
    Ok(store.model_identity()?.as_deref() == Some(identity.as_str()))
}

/// Records `embedder`'s identity on a fresh database (one with no identity yet), so its
/// chunks can be embedded; leaves an existing record alone, even a mismatching one, so a model
/// change still needs a confirmed re-embed. Called at startup and before imports.
///
/// # Arguments
/// * `store` - Store whose identity record may be written.
/// * `embedder` - The configured embedder; locked briefly to read its identity.
///
/// # Errors
/// * [`Error::Storage`](crate::error::Error::Storage) - Reading or writing the identity fails.
pub fn adopt_if_fresh(store: &dyn NoteStore, embedder: &SharedEmbedder) -> Result<()> {
    if store.model_identity()?.is_none() {
        let identity = lock(embedder).identity();
        store.reset_embeddings(&identity)?;
    }
    Ok(())
}

/// Chunks notes stored without chunks, then embeds every pending chunk in the calling thread until
/// none is left; does nothing if the stored model identity differs from `embedder`'s. Locks the
/// embedder per chunk. A failed chunk (error or panic) is skipped for the rest of this call and its
/// failure recorded. After [`OUTAGE_STREAK`] failures in a row the embedder is probed: if the probe
/// fails it is an outage, so the pass stops and charges none of them. The store stops offering a
/// chunk after a bounded number of recorded failures.
///
/// # Arguments
/// * `store` - Store supplying pending chunks and receiving embeddings.
/// * `embedder` - The configured shared embedder.
/// * `progress` - Called with the running count after each stored vector.
///
/// # Returns
/// The number of embeddings stored (stale writes for replaced chunks not counted).
///
/// # Errors
/// * [`Error::Storage`](crate::error::Error::Storage) - Any store call fails; embeddings stored
///   before it are kept.
pub fn drain(store: &dyn NoteStore, embedder: &SharedEmbedder, progress: &mut dyn FnMut(usize)) -> Result<usize> {
    drain_until(store, embedder, &AtomicBool::new(false), progress)
}

/// Same as [`drain`], but checks `stop` before each chunk and returns early once it is set;
/// used by the background drainer so dropping it does not wait for the whole backlog.
///
/// # Arguments
/// * `store` - Store supplying pending chunks and receiving embeddings.
/// * `embedder` - The configured shared embedder; locked per chunk.
/// * `stop` - Set by the drainer's owner to request an early return.
/// * `progress` - Called with the running count after each stored vector.
///
/// # Returns
/// The number of embeddings stored before finishing or stopping.
///
/// # Errors
/// * [`Error::Storage`](crate::error::Error::Storage) - Any store call fails.
pub(crate) fn drain_until(
    store: &dyn NoteStore,
    embedder: &SharedEmbedder,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(usize),
) -> Result<usize> {
    if !identity_matches(store, embedder)? {
        return Ok(0);
    }
    chunk_missing(store, embedder)?;
    let mut done = 0;
    let mut cursor = ChunkId(0);
    let mut streak: Vec<ChunkId> = Vec::new();
    loop {
        let batch = store.pending_chunks(cursor, BATCH)?;
        if batch.is_empty() {
            charge(store, &mut streak)?;
            return Ok(done);
        }
        for c in batch {
            cursor = c.id;
            if stop.load(Ordering::SeqCst) {
                return Ok(done);
            }
            match guarded(embedder, |e| embedding::embed(e, &c.text, Kind::Document)) {
                Ok(vector) => {
                    charge(store, &mut streak)?;
                    if store.set_embedding(c.id, &vector)? {
                        done += 1;
                        progress(done);
                    }
                }
                Err(err) => {
                    streak.push(c.id);
                    tracing::warn!(chunk = c.id.0, "embedding chunk failed: {err}");
                    if streak.len() >= OUTAGE_STREAK {
                        if guarded(embedder, |e| embedding::embed(e, PROBE_TEXT, Kind::Document)).is_err() {
                            tracing::warn!("embedder looks down; stopping this pass without charging attempts");
                            return Ok(done);
                        }
                        charge(store, &mut streak)?;
                    }
                }
            }
        }
    }
}

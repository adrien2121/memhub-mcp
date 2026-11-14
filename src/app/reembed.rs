//! Detects an embedder change and re-embeds, only once the user has confirmed.
//!
//! A mismatch never touches the stored embeddings or identity: switching the config back restores
//! vector search with no work.

use std::sync::{Arc, Mutex};

use crate::app::embed_worker::{self, Wake};
use crate::domain::{EmbedderChange, ReembedStatus};
use crate::error::{Error, Result};
use crate::ports::embedder::SharedEmbedder;
use crate::ports::store::NoteStore;

/// Measured embedding cost per chunk, in seconds.
const SECONDS_PER_CHUNK: f64 = 0.085;

/// Detects whether the configured embedder differs from the one that made the stored
/// embeddings. Read-only; locks the embedder briefly for its identity.
///
/// # Arguments
/// * `store` - Store holding the recorded identity and chunk count.
/// * `embedder` - The configured embedder.
///
/// # Returns
/// The change with names and cost estimate, or `None` when the identities match or the store
/// has no recorded identity yet.
///
/// # Errors
/// * [`Error::Storage`] - Reading the identity or counting chunks fails.
pub fn change(store: &dyn NoteStore, embedder: &SharedEmbedder) -> Result<Option<EmbedderChange>> {
    let Some(stored) = store.model_identity()? else { return Ok(None) };
    let configured = embed_worker::lock(embedder).identity();
    change_between(store, &stored, &configured)
}

/// Describes the change from the `stored` identity fingerprint to the `configured` one, with
/// the chunk count and an estimated re-embed time (0.085 s per chunk, rounded up). Shows only
/// the `model=` fields when the models differ, full fingerprints when only a setting differs.
///
/// # Arguments
/// * `store` - Store whose chunks would be re-embedded.
/// * `stored` - Identity fingerprint recorded with the embeddings.
/// * `configured` - Identity fingerprint of the configured embedder.
///
/// # Returns
/// The change, or `None` when the fingerprints are equal.
///
/// # Errors
/// * [`Error::Storage`] - Counting chunks fails.
pub fn change_between(store: &dyn NoteStore, stored: &str, configured: &str) -> Result<Option<EmbedderChange>> {
    if stored == configured {
        return Ok(None);
    }
    let chunks = store.chunk_count()?;
    let (s, c) = (model_id(stored), model_id(configured));
    let (stored, configured) = if s == c { (stored, configured) } else { (s, c) };
    Ok(Some(EmbedderChange {
        stored: stored.into(),
        configured: configured.into(),
        chunks,
        est_seconds: (chunks as f64 * SECONDS_PER_CHUNK).ceil(),
    }))
}

/// Extracts the model name from an identity fingerprint (space-separated `key=value` fields).
///
/// # Arguments
/// * `identity` - Embedder identity fingerprint.
///
/// # Returns
/// The value of the `model=` field, or the whole fingerprint when there is none.
fn model_id(identity: &str) -> &str {
    identity.split(' ').find_map(|f| f.strip_prefix("model=")).unwrap_or(identity)
}

/// Runs a confirmed re-embed in the background drainer.
pub struct ReembedService {
    store: Arc<dyn NoteStore>,
    embedder: Option<SharedEmbedder>,
    wake: Option<Wake>,
    /// Serialises calls, so two confirmations never requeue twice.
    running: Mutex<()>,
}

impl ReembedService {
    /// Creates the service.
    ///
    /// # Arguments
    /// * `store` - Store to inspect and requeue.
    /// * `embedder` - Configured embedder; `None` makes every call report status only.
    /// * `wake` - Wakes the background drainer after a requeue; `None` leaves chunks pending.
    ///
    /// # Returns
    /// The service, not yet running.
    pub fn new(store: Arc<dyn NoteStore>, embedder: Option<SharedEmbedder>, wake: Option<Wake>) -> Self {
        Self { store, embedder, wake, running: Mutex::new(()) }
    }

    /// Reports the model change. With `confirm` and a change, drops all embeddings and re-chunks
    /// every note under the configured identity; with `confirm` and no change, clears failed
    /// attempts and chunks notes stored without chunks. Either way it then wakes the drainer and
    /// returns before any chunk is embedded. Holds an internal mutex so concurrent confirmations
    /// never requeue twice.
    ///
    /// # Arguments
    /// * `confirm` - The user agreed to discard the old embeddings.
    ///
    /// # Returns
    /// Whether a re-embed started, the detected change (if any), and the pending and failed
    /// embedding counts after the call.
    ///
    /// # Errors
    /// * [`Error::Embedder`] - The internal mutex is poisoned, or tokenising a note fails.
    /// * [`Error::Storage`] - Any store call fails.
    pub fn reembed(&self, confirm: bool) -> Result<ReembedStatus> {
        let _running = self.running.lock().map_err(|_| Error::Embedder("reembed lock poisoned".into()))?;
        let changed = match &self.embedder {
            Some(e) => change(self.store.as_ref(), e)?,
            None => None,
        };
        let mut started = false;
        if let (true, Some(e)) = (confirm, &self.embedder) {
            started = if changed.is_some() {
                embed_worker::requeue(self.store.as_ref(), e)? > 0
            } else {
                let reset = self.store.reset_failures()?;
                reset + embed_worker::chunk_missing(self.store.as_ref(), e)? > 0
            };
            if let Some(wake) = &self.wake {
                wake();
            }
        }
        Ok(ReembedStatus {
            started,
            embedder_changed: changed,
            pending_embeddings: self.store.pending_count()?,
            failed_embeddings: self.store.failed_count()?,
        })
    }
}

//! [`EmbeddingQueue`] for [`FakeStore`]: a test-filled chunk queue with injectable failures.

use std::sync::atomic::Ordering;

use super::FakeStore;
use crate::domain::{Chunk, ChunkId, NoteId};
use crate::error::{Error, Result};
use crate::ports::store::EmbeddingQueue;

impl EmbeddingQueue for FakeStore {
    /// Counts the call in `pending_calls`; while `pending_failures` is above zero, decrements
    /// it and fails, otherwise returns up to `limit` queued `chunks` with ids above `after`,
    /// adding them to `fetched`.
    ///
    /// # Arguments
    /// * `after` - Keyset cursor.
    /// * `limit` - Batch size.
    ///
    /// # Returns
    /// The first `limit` queued chunks past `after`.
    ///
    /// # Errors
    /// * [`Error::Storage`] - An injected failure remains (`database is locked`).
    fn pending_chunks(&self, after: ChunkId, limit: usize) -> Result<Vec<Chunk>> {
        self.pending_calls.fetch_add(1, Ordering::SeqCst);
        let failing = self.pending_failures.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
        if failing.is_ok() {
            return Err(Error::Storage("database is locked".into()));
        }
        let chunks = self.chunks.lock().map_err(|e| Error::Storage(e.to_string()))?;
        let batch: Vec<Chunk> = chunks.iter().filter(|c| c.id.0 > after.0).take(limit).cloned().collect();
        self.fetched.fetch_add(batch.len(), Ordering::SeqCst);
        Ok(batch)
    }

    /// Reports an empty queue.
    ///
    /// # Returns
    /// Always 0.
    ///
    /// # Errors
    /// Never fails.
    fn pending_count(&self) -> Result<usize> {
        Ok(0)
    }

    /// Ignores the failure.
    ///
    /// # Arguments
    /// * `_` - Failed chunk (ignored).
    ///
    /// # Returns
    /// Always `false`: the chunk is never given up.
    ///
    /// # Errors
    /// Never fails.
    fn record_embed_failure(&self, _: ChunkId) -> Result<bool> {
        Ok(false)
    }

    /// Reports no permanently failed chunks.
    ///
    /// # Returns
    /// Always 0.
    ///
    /// # Errors
    /// Never fails.
    fn failed_count(&self) -> Result<usize> {
        Ok(0)
    }

    /// Reports no stored chunks.
    ///
    /// # Returns
    /// Always 0.
    ///
    /// # Errors
    /// Never fails.
    fn chunk_count(&self) -> Result<usize> {
        Ok(0)
    }

    /// Removes the chunk from the queue.
    ///
    /// # Arguments
    /// * `chunk` - Embedded chunk; the vector is discarded.
    ///
    /// # Returns
    /// `true` if the chunk was queued, `false` as for a stale write.
    ///
    /// # Errors
    /// * [`Error::Storage`] - The chunk mutex is poisoned.
    fn set_embedding(&self, chunk: ChunkId, _: &[f32]) -> Result<bool> {
        let mut chunks = self.chunks.lock().map_err(|e| Error::Storage(e.to_string()))?;
        let before = chunks.len();
        chunks.retain(|c| c.id != chunk);
        Ok(chunks.len() < before)
    }

    /// Reads the identity tests put in the public `identity` field.
    ///
    /// # Returns
    /// The recorded identity, or `None` if unset.
    ///
    /// # Errors
    /// * [`Error::Storage`] - The identity mutex is poisoned.
    fn model_identity(&self) -> Result<Option<String>> {
        self.identity.lock().map(|i| i.clone()).map_err(|e| Error::Storage(e.to_string()))
    }

    /// Does nothing: neither drops embeddings nor records the identity.
    ///
    /// # Arguments
    /// * `_` - New identity (ignored).
    ///
    /// # Errors
    /// Never fails.
    fn reset_embeddings(&self, _: &str) -> Result<()> {
        Ok(())
    }

    /// Reports no chunkless notes.
    ///
    /// # Returns
    /// Always empty.
    ///
    /// # Errors
    /// Never fails.
    fn chunkless_notes(&self) -> Result<Vec<NoteId>> {
        Ok(Vec::new())
    }

    /// Does nothing: the fake has no chunks.
    ///
    /// # Returns
    /// Always `0`.
    ///
    /// # Errors
    /// Never fails.
    fn reset_failures(&self) -> Result<usize> {
        Ok(0)
    }
}

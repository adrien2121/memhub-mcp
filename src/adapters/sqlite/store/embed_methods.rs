//! `EmbeddingQueue` impl: pending chunks, storing embeddings, failure counters and model identity.

use super::super::{SqliteStore, chunks, meta};
use crate::domain::{Chunk, ChunkId, NoteId};
use crate::error::Result;
use crate::ports::store::EmbeddingQueue;

impl EmbeddingQueue for SqliteStore {
    /// Returns the oldest chunks of live notes still waiting for an embedding, skipping chunks that
    /// already used up their failed attempts. Runs on a pooled read connection.
    ///
    /// # Arguments
    /// * `after` - keyset cursor: only chunks with a greater id are returned.
    /// * `limit` - maximum number of chunks returned.
    ///
    /// # Returns
    /// Up to `limit` chunks with ids above `after`, in ascending chunk-id order.
    ///
    /// # Errors
    /// * [`Error::InvalidInput`](crate::error::Error::InvalidInput) - `limit` exceeds `i64::MAX`.
    /// * [`Error::Storage`](crate::error::Error::Storage) - the pool fails or the SQLite query fails.
    fn pending_chunks(&self, after: ChunkId, limit: usize) -> Result<Vec<Chunk>> {
        self.read(|c| chunks::pending(c, after, limit))
    }

    /// Stores `vector` on `chunk` in one `IMMEDIATE` write transaction; if stored, adds it to the
    /// in-memory vector tier after commit (which may rebuild the index once the count passes the
    /// exact-scan threshold).
    ///
    /// # Arguments
    /// * `chunk` - the chunk to update.
    /// * `vector` - the embedding; must be finite and not all zeros.
    ///
    /// # Returns
    /// `true` if stored; `false` if the chunk was deleted or already embedded meanwhile.
    ///
    /// # Errors
    /// * [`Error::InvalidInput`](crate::error::Error::InvalidInput) - `vector` has a non-finite value or is all zeros.
    /// * [`Error::Storage`](crate::error::Error::Storage) - a lock is poisoned, SQLite fails, or the index rejects the vector.
    fn set_embedding(&self, chunk: ChunkId, vector: &[f32]) -> Result<bool> {
        let (stored, build) = self.write(
            |tx| chunks::set_embedding(tx, chunk, vector),
            |_, stored| Ok((stored, if stored { self.remember(chunk, vector)? } else { None })),
        )?;
        if let Some(id) = build {
            self.build_index(id)?;
        }
        Ok(stored)
    }

    /// Counts chunks of live notes still waiting for an embedding, on a pooled read connection.
    ///
    /// # Returns
    /// The number of unembedded chunks that are still retried.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the pool fails, the query fails or the count is negative.
    fn pending_count(&self) -> Result<usize> {
        self.read(chunks::pending_count)
    }

    /// Increments the failed-attempt counter of `chunk` in one `IMMEDIATE` write transaction, if the
    /// chunk still has no embedding.
    ///
    /// # Arguments
    /// * `chunk` - the chunk whose embedding failed.
    ///
    /// # Returns
    /// `true` exactly when this failure reaches the attempt limit, so the chunk is no longer retried.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the writer lock is poisoned or SQLite fails.
    fn record_embed_failure(&self, chunk: ChunkId) -> Result<bool> {
        self.write(|tx| chunks::record_embed_failure(tx, chunk), |_, last| Ok(last))
    }

    /// Counts chunks of live notes that gave up after the attempt limit, on a pooled read connection.
    ///
    /// # Returns
    /// The number of chunks no longer retried.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the pool fails, the query fails or the count is negative.
    fn failed_count(&self) -> Result<usize> {
        self.read(chunks::failed_count)
    }

    /// Counts all chunks of live notes, embedded or not, on a pooled read connection.
    ///
    /// # Returns
    /// The total chunk count.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the pool fails, the query fails or the count is negative.
    fn chunk_count(&self) -> Result<usize> {
        self.read(chunks::chunk_count)
    }

    /// Reads the embedding-model identity recorded in the `meta` table, on a pooled read connection.
    ///
    /// # Returns
    /// The identity the stored vectors were made with, or `None` if none was ever recorded.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the pool fails or the SQLite query fails.
    fn model_identity(&self) -> Result<Option<String>> {
        self.read(meta::model_identity)
    }

    /// Clears all embeddings and failure counters and records `identity` in one `IMMEDIATE` write
    /// transaction, then reloads the vector tier (now empty) while holding the writer lock.
    ///
    /// # Arguments
    /// * `identity` - the new embedding-model identity.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - a lock is poisoned or SQLite fails.
    fn reset_embeddings(&self, identity: &str) -> Result<()> {
        self.write(|tx| meta::reset_embeddings(tx, identity), |c, ()| self.reload_tier(c))
    }

    /// Lists live, non-blank notes without chunks, on a pooled read connection.
    ///
    /// # Returns
    /// Their ids, ascending.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the pool fails or the query fails.
    fn chunkless_notes(&self) -> Result<Vec<NoteId>> {
        self.read(chunks::chunkless_notes)
    }

    /// Clears failed-attempt counts of unembedded chunks in one `IMMEDIATE` write transaction.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - the writer lock is poisoned or SQLite fails.
    fn reset_failures(&self) -> Result<usize> {
        self.write(|tx| chunks::reset_failures(tx), |_, n| Ok(n))
    }
}

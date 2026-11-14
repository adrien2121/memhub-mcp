//! Embedding bookkeeping: the pending queue, failures, stored vectors and model identity.

use crate::domain::{Chunk, ChunkId, NoteId};
use crate::error::Result;

/// Embedding queue and vector storage; part of [`super::NoteStore`].
pub trait EmbeddingQueue {
    /// Fetches chunks still awaiting an embedding, for the background embedding worker. Pages
    /// by id so a caller can walk the queue once, past chunks it failed to embed.
    ///
    /// # Arguments
    /// * `after` - Keyset cursor: only chunks with a greater id are returned (`ChunkId(0)` for all).
    /// * `limit` - Maximum number of chunks.
    ///
    /// # Returns
    /// Up to `limit` pending chunks with ids above `after`, in ascending id order.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn pending_chunks(&self, after: ChunkId, limit: usize) -> Result<Vec<Chunk>>;

    /// Counts chunks of live notes still awaiting an embedding, excluding failed ones.
    ///
    /// # Returns
    /// Number of pending chunks.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn pending_count(&self) -> Result<usize>;

    /// Records one failed embedding attempt; after a bounded number of failures the chunk leaves
    /// [`Self::pending_chunks`] and counts in [`Self::failed_count`] until re-embedded.
    ///
    /// # Arguments
    /// * `chunk` - Chunk whose embedding failed.
    ///
    /// # Returns
    /// `true` if this failure was the chunk's last allowed attempt.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn record_embed_failure(&self, chunk: ChunkId) -> Result<bool>;

    /// Counts chunks of live notes whose embedding failed too often to retry.
    ///
    /// # Returns
    /// Number of failed chunks.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn failed_count(&self) -> Result<usize>;

    /// Counts chunks of live notes, embedded or not.
    ///
    /// # Returns
    /// Total number of chunks.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn chunk_count(&self) -> Result<usize>;

    /// Stores a chunk's embedding if the chunk still exists and has none yet.
    ///
    /// # Arguments
    /// * `chunk` - Chunk the vector belongs to.
    /// * `vector` - The normalised embedding.
    ///
    /// # Returns
    /// `false` if the chunk was replaced or already embedded (a stale write that is dropped).
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn set_embedding(&self, chunk: ChunkId, vector: &[f32]) -> Result<bool>;

    /// Reads the identity of the embedder the stored vectors were made with.
    ///
    /// # Returns
    /// The identity string, or `None` on a fresh database.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn model_identity(&self) -> Result<Option<String>>;

    /// Drops every embedding and records the new embedder identity, in one transaction, so all
    /// chunks are re-embedded by the new model.
    ///
    /// # Arguments
    /// * `identity` - Identity of the embedder now in use.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn reset_embeddings(&self, identity: &str) -> Result<()>;

    /// Lists live notes with a non-blank body but no chunks (written while no embedder was
    /// configured), so they can be chunked once one is.
    ///
    /// # Returns
    /// Their ids, ascending.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn chunkless_notes(&self) -> Result<Vec<NoteId>>;

    /// Clears every unembedded chunk's failed-attempt count, so failed chunks become pending again.
    ///
    /// # Returns
    /// The number of chunks cleared.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn reset_failures(&self) -> Result<usize>;
}

//! `Embedder` port: turns text into a vector; `app::embedding` L2-normalises it.

use std::sync::{Arc, Mutex};

use crate::error::Result;

/// What the text is, so the adapter can apply its own prefixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A search query.
    Query,
    /// A stored chunk of a note.
    Document,
}

/// Text embedding model.
pub trait Embedder: Send {
    /// Runs the model on one text; may take the model's inference time.
    ///
    /// # Arguments
    /// * `text` - Text to embed.
    /// * `kind` - Whether `text` is a query or a stored chunk, so the adapter applies its prefix.
    ///
    /// # Returns
    /// A vector of length [`Embedder::dim`] at any scale; `app::embedding` L2-normalises it.
    ///
    /// # Errors
    /// * [`crate::error::Error::Embedder`] - Tokenising or inference fails.
    fn embed(&mut self, text: &str, kind: Kind) -> Result<Vec<f32>>;

    /// Reports the width of the vectors this model produces.
    ///
    /// # Returns
    /// Number of `f32` values per embedding.
    fn dim(&self) -> usize;

    /// Fingerprints the model, its files, pooling and prefixes; compared with the identity stored
    /// in the database `meta` table to detect vectors made by a different model.
    ///
    /// # Returns
    /// A stable string that changes whenever any of those inputs change.
    fn identity(&self) -> String;

    /// Hands out the model's tokenizer, usable without holding the embedder lock.
    ///
    /// # Returns
    /// A shared handle to the tokenizer.
    fn tokenizer(&self) -> Arc<dyn Tokenizer>;
}

/// Tokenizer of an [`Embedder`], shared across threads so chunking never waits on inference.
pub trait Tokenizer: Send + Sync {
    /// Finds the byte span of each content token of `text`, without special tokens or prefixes.
    ///
    /// # Arguments
    /// * `text` - Text to tokenise.
    ///
    /// # Returns
    /// Byte spans `[start, end)` into `text`, in order.
    ///
    /// # Errors
    /// * [`crate::error::Error::Embedder`] - Tokenising fails.
    fn token_spans(&self, text: &str) -> Result<Vec<(usize, usize)>>;

    /// Reports the content-token window the chunker starts from for one [`Kind::Document`] input.
    ///
    /// # Returns
    /// Upper bound on content tokens in one stored chunk; [`Tokenizer::document_overflow`] may
    /// shrink a chunk further.
    fn max_tokens(&self) -> usize;

    /// Counts how many tokens of `text`, once the adapter's document prefix is applied, exceed
    /// what the embedder feeds the model for one [`Kind::Document`] input.
    ///
    /// The default suits tokenizers without a document prefix, whose [`Tokenizer::max_tokens`]
    /// is already exact.
    ///
    /// # Arguments
    /// * `text` - Candidate chunk text.
    ///
    /// # Returns
    /// Tokens the embedder would truncate; 0 when `text` fits whole (always 0 by default).
    ///
    /// # Errors
    /// * [`crate::error::Error::Embedder`] - Tokenising fails.
    fn document_overflow(&self, text: &str) -> Result<usize> {
        let _ = text;
        Ok(0)
    }
}

/// One embedder shared by every session; never lock it while holding the DB write lock.
pub type SharedEmbedder = Arc<Mutex<Box<dyn Embedder>>>;

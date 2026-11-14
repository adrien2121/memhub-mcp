//! Unit tests for [`super::embed_worker::drain`] with failing chunks.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::embed_worker::drain;
use crate::app::fake::FakeStore;
use crate::domain::{Chunk, ChunkId, NoteId};
use crate::error::{Error, Result};
use crate::ports::embedder::{Embedder, Kind, SharedEmbedder, Tokenizer};

/// Embedder that fails on any text containing `bad` and returns `[1.0]` otherwise.
struct Picky;

impl Tokenizer for Picky {
    /// Ignores the text.
    ///
    /// # Returns
    /// No token spans.
    ///
    /// # Errors
    /// Never fails.
    fn token_spans(&self, _: &str) -> Result<Vec<(usize, usize)>> {
        Ok(Vec::new())
    }
    /// Reports a window of one token.
    ///
    /// # Returns
    /// 1.
    fn max_tokens(&self) -> usize {
        1
    }
}

impl Embedder for Picky {
    /// Embeds `text` unless it contains `bad`.
    ///
    /// # Returns
    /// `[1.0]`.
    ///
    /// # Errors
    /// * [`Error::Storage`] - `text` contains `bad`.
    fn embed(&mut self, text: &str, _: Kind) -> Result<Vec<f32>> {
        if text.contains("bad") {
            return Err(Error::Storage("cannot embed".into()));
        }
        Ok(vec![1.0])
    }
    /// Reports dimension 1.
    ///
    /// # Returns
    /// 1.
    fn dim(&self) -> usize {
        1
    }
    /// Names the model.
    ///
    /// # Returns
    /// `picky`.
    fn identity(&self) -> String {
        "picky".into()
    }
    /// Hands out this fake's tokenizer.
    ///
    /// # Returns
    /// A fresh [`Picky`].
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        Arc::new(Picky)
    }
}

/// A drain over 300 chunks, every other one failing (never three in a row, so no outage),
/// fetches each pending row once and still embeds every healthy chunk.
#[test]
fn failing_chunks_are_fetched_once_per_drain() -> Result<()> {
    let store = FakeStore::default();
    *store.identity.lock().map_err(|e| Error::Storage(e.to_string()))? = Some("picky".into());
    let chunks = (1..=300).map(|i| Chunk {
        id: ChunkId(i),
        note_id: NoteId(1),
        seq: 0,
        text: if i % 2 == 0 { "bad".into() } else { "good".into() },
    });
    store.chunks.lock().map_err(|e| Error::Storage(e.to_string()))?.extend(chunks);
    let embedder: SharedEmbedder = Arc::new(Mutex::new(Box::new(Picky)));
    let done = drain(&store, &embedder, &mut |_| {})?;
    assert_eq!(done, 150, "every healthy chunk is embedded");
    assert_eq!(store.fetched.load(Ordering::SeqCst), 300, "each pending row is fetched once");
    Ok(())
}

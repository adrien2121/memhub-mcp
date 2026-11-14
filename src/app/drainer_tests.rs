//! Unit tests for [`super`].

use std::sync::Mutex;
use std::time::Instant;

use super::*;
use crate::app::fake::FakeStore;
use crate::error::{Error, Result};
use crate::ports::embedder::{Embedder, Kind, Tokenizer};

/// Embedder and tokenizer that produce no tokens, the vector `[1.0]` and identity `unit`.
struct Unit;

impl Tokenizer for Unit {
    /// Ignores the text.
    ///
    /// # Arguments
    /// * `_` - Text to tokenize (unused).
    ///
    /// # Returns
    /// No token spans.
    ///
    /// # Errors
    /// Never fails.
    fn token_spans(&self, _: &str) -> Result<Vec<(usize, usize)>> {
        Ok(Vec::new())
    }
    /// Reports the window size; irrelevant since no tokens are produced.
    ///
    /// # Returns
    /// A window of one token.
    fn max_tokens(&self) -> usize {
        1
    }
}

impl Embedder for Unit {
    /// Ignores the input and returns a fixed vector.
    ///
    /// # Arguments
    /// * `_` - Text and kind (unused).
    ///
    /// # Returns
    /// `[1.0]`, already unit length.
    ///
    /// # Errors
    /// Never fails.
    fn embed(&mut self, _: &str, _: Kind) -> Result<Vec<f32>> {
        Ok(vec![1.0])
    }
    /// Reports the vector size, matching [`Unit::embed`].
    ///
    /// # Returns
    /// Dimension 1.
    fn dim(&self) -> usize {
        1
    }
    /// Names the model; the test stores the same name so drains are not skipped as mismatched.
    ///
    /// # Returns
    /// The fixed identity `unit`.
    fn identity(&self) -> String {
        "unit".into()
    }
    /// Hands out this fake's tokenizer.
    ///
    /// # Returns
    /// A fresh [`Unit`] as the tokenizer.
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        Arc::new(Unit)
    }
}

/// Proves a drain that hits a storage error is retried on its own, with no new write to wake
/// it: the fake store fails `pending_chunks` twice, and the test waits (up to 5 s) for exactly
/// three calls, two failed drains then one that succeeds.
#[test]
fn storage_error_is_retried_without_a_wake() -> Result<()> {
    let store = Arc::new(FakeStore::default());
    *store.identity.lock().map_err(|e| Error::Storage(e.to_string()))? = Some("unit".into());
    store.pending_failures.store(2, Ordering::SeqCst);
    let embedder: SharedEmbedder = Arc::new(Mutex::new(Box::new(Unit)));
    let drainer = spawn(Arc::clone(&store) as Arc<dyn NoteStore>, embedder);
    let deadline = Instant::now() + Duration::from_secs(5);
    while store.pending_calls.load(Ordering::SeqCst) < 3 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(drainer);
    assert_eq!(store.pending_calls.load(Ordering::SeqCst), 3, "two failed drains, then one that succeeds");
    Ok(())
}

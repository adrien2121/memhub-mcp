//! Vectors are L2-normalised at the app boundary whatever the embedder returns; unusable ones are rejected.

mod common;

use std::sync::{Arc, Mutex};

use memhub_mcp::app::embed_worker;
use memhub_mcp::app::nearest::nearest;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::{NewNote, SearchFilter, SearchRequest};
use memhub_mcp::error::Result;
use memhub_mcp::ports::embedder::{Embedder, Kind, SharedEmbedder, Tokenizer};
use memhub_mcp::ports::store::NoteStore;

/// Returns the same raw vector for every text.
struct Fixed(Vec<f32>);

impl Embedder for Fixed {
    /// Returns a clone of the stored raw vector, ignoring the text and kind.
    ///
    /// # Arguments
    /// * `_` - input text, ignored.
    /// * `_` - embedding kind, ignored.
    ///
    /// # Returns
    /// The stored vector.
    ///
    /// # Errors
    /// * Never fails; always returns `Ok`.
    fn embed(&mut self, _: &str, _: Kind) -> Result<Vec<f32>> {
        Ok(self.0.clone())
    }

    /// Returns the length of the stored vector.
    ///
    /// # Returns
    /// The vector length.
    fn dim(&self) -> usize {
        self.0.len()
    }

    /// Returns the fixed identity string `fixed`.
    ///
    /// # Returns
    /// `"fixed"`.
    fn identity(&self) -> String {
        "fixed".into()
    }

    /// Returns a shared `FakeTokenizer` allowing up to 64 tokens.
    ///
    /// # Returns
    /// The tokenizer behind an `Arc`.
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        Arc::new(common::FakeTokenizer { max_tokens: 64 })
    }
}

/// Creates a named test store, adopts an embedder that returns `vector`, and writes one note with body `zebra` so it is embedded later.
///
/// Side effects: creates the store through `common::store`.
///
/// # Arguments
/// * `name` - store name passed to `common::store`.
/// * `vector` - raw vector the embedder returns for every text.
///
/// # Returns
/// The store and the shared embedder.
///
/// # Errors
/// * Propagates store creation, adoption, or write errors.
fn setup(name: &str, vector: Vec<f32>) -> Result<(Arc<dyn NoteStore>, SharedEmbedder)> {
    let store: Arc<dyn NoteStore> = common::store(name)?;
    let e: SharedEmbedder = Arc::new(Mutex::new(Box::new(Fixed(vector))));
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&e)), None);
    notes.write(NewNote { title: "t".into(), body: "zebra".into(), tags: Vec::new() }, false)?;
    Ok((store, e))
}

/// Checks that a non-unit embedder output is stored L2-normalised: after draining, a nearest-neighbour query along `[0.6, 0.8]` scores the stored vector at cosine 1.
///
/// Setup: the embedder returns `[3, 4]`. Asserts that one vector is stored and that its cosine is 1 within 1e-5.
///
/// # Errors
/// * Propagates store, drain, or nearest-query errors.
#[test]
fn stored_vectors_are_unit_length() -> Result<()> {
    let (store, e) = setup("norm_stored", vec![3.0, 4.0])?;
    assert_eq!(embed_worker::drain(store.as_ref(), &e, &mut |_| {})?, 1);
    let hits = nearest(store.as_ref(), &SearchFilter::default(), &[0.6, 0.8], 10)?;
    assert!((hits[0].1 - 1.0).abs() < 1e-5, "cosine must be 1, got {}", hits[0].1);
    Ok(())
}

/// Checks that an all-zero embedder output is rejected: draining stores nothing, the chunk stays pending, and a search falls back to keyword-only.
///
/// Setup: the embedder returns `[0, 0]`. Asserts that drain returns 0, one chunk stays pending, and a `zebra` search is keyword-only.
///
/// # Errors
/// * Propagates store, drain, or search errors.
#[test]
fn zero_vector_is_rejected_for_storage_and_queries() -> Result<()> {
    let (store, e) = setup("norm_zero", vec![0.0, 0.0])?;
    assert_eq!(embed_worker::drain(store.as_ref(), &e, &mut |_| {})?, 0);
    assert_eq!(store.pending_count()?, 1, "nothing stored");
    let svc = SearchService::new(Arc::clone(&store), Some(e));
    let req = SearchRequest { query: "zebra".into(), limit: 10, filter: SearchFilter::default() };
    assert!(svc.search(&req)?.keyword_only, "zero query vector falls back to keyword search");
    Ok(())
}

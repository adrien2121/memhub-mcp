//! Shared test fixtures: a deterministic fake embedder and a temp store.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::error::{Error, Result};
use memhub_mcp::ports::embedder::{Embedder, Kind, SharedEmbedder, Tokenizer};

const DIM: usize = 4096;

/// Whitespace tokens; vectors hash character trigrams, so near-spellings stay close.
pub struct FakeEmbedder {
    pub identity: String,
    pub tokenizer: Arc<FakeTokenizer>,
    pub down: Arc<AtomicBool>,
}

impl Embedder for FakeEmbedder {
    /// Embeds `text` as a unit-length vector by hashing character trigrams of each word.
    ///
    /// Each word is lowercased and wrapped in `^` and `$`. Every 3-character window increments one of `DIM` buckets, so words with similar spelling land in similar vectors. The `_` kind argument is ignored. While the `down` switch is set, the call fails to simulate an unavailable model.
    ///
    /// # Arguments
    /// * `text` - text to embed, split on whitespace.
    /// * `_` - embedding kind (document or query); ignored.
    ///
    /// # Returns
    /// A vector of length `DIM` scaled to norm 1. Text with no words yields an all-zero vector.
    ///
    /// # Errors
    /// * `Error::Embedder` when the `down` switch is set.
    fn embed(&mut self, text: &str, _: Kind) -> Result<Vec<f32>> {
        if self.down.load(Ordering::SeqCst) {
            return Err(Error::Embedder("down".into()));
        }
        let mut v = vec![0f32; DIM];
        for word in text.split_whitespace() {
            let w: Vec<char> = format!("^{}$", word.to_lowercase()).chars().collect();
            for t in w.windows(3) {
                let h = t.iter().fold(7usize, |h, c| h.wrapping_mul(31).wrapping_add(*c as usize));
                v[h % DIM] += 1.0;
            }
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(f32::MIN_POSITIVE);
        Ok(v.into_iter().map(|x| x / norm).collect())
    }

    /// Returns the embedding width.
    ///
    /// # Returns
    /// `DIM`, which is 4096.
    fn dim(&self) -> usize {
        DIM
    }

    /// Returns the identity string this embedder was constructed with.
    ///
    /// # Returns
    /// A copy of the `identity` field.
    fn identity(&self) -> String {
        self.identity.clone()
    }

    /// Returns the shared tokenizer as a trait object.
    ///
    /// # Returns
    /// An `Arc<dyn Tokenizer>` pointing at the same `FakeTokenizer` that this embedder holds.
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        Arc::clone(&self.tokenizer) as Arc<dyn Tokenizer>
    }
}

/// Splits on whitespace and reports `max_tokens` as the chunk limit. The limit is returned but not enforced by the splitting.
pub struct FakeTokenizer {
    pub max_tokens: usize,
}

impl Tokenizer for FakeTokenizer {
    /// Returns the byte span of each whitespace-separated word in `text`.
    ///
    /// Each span is `(start, end)` with `end` exclusive. A word that ends at the end of the string is closed by a sentinel space appended to the scan, so it is included.
    ///
    /// # Arguments
    /// * `text` - text to split.
    ///
    /// # Returns
    /// Spans in order of appearance; empty when `text` has no non-whitespace characters.
    ///
    /// # Errors
    /// * Never fails. The `Result` return type is required by the `Tokenizer` trait.
    fn token_spans(&self, text: &str) -> Result<Vec<(usize, usize)>> {
        let mut spans = Vec::new();
        let mut start = None;
        for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
            match (c.is_whitespace(), start) {
                (true, Some(s)) => {
                    spans.push((s, i));
                    start = None;
                }
                (false, None) => start = Some(i),
                _ => {}
            }
        }
        Ok(spans)
    }

    /// Returns the `max_tokens` value this tokenizer was constructed with.
    ///
    /// # Returns
    /// The stored limit. This fake does not enforce it when splitting.
    fn max_tokens(&self) -> usize {
        self.max_tokens
    }
}

/// Builds a `SharedEmbedder` around a new `FakeEmbedder` and returns it with its kill switch.
///
/// # Arguments
/// * `identity` - identity string the embedder reports.
/// * `max_tokens` - value the fake tokenizer reports from `max_tokens()`.
///
/// # Returns
/// The shared embedder and an `Arc<AtomicBool>`; storing `true` makes every `embed` call fail.
pub fn embedder(identity: &str, max_tokens: usize) -> (SharedEmbedder, Arc<AtomicBool>) {
    let down = Arc::new(AtomicBool::new(false));
    let tokenizer = Arc::new(FakeTokenizer { max_tokens });
    let fake = FakeEmbedder { identity: identity.into(), tokenizer, down: Arc::clone(&down) };
    (Arc::new(Mutex::new(Box::new(fake))), down)
}

/// Opens a new `SqliteStore` on `m.db` inside a per-process temp directory.
///
/// Removes any earlier directory with the same name first, so each call starts with an empty database. The store uses the `porter unicode61` full-text tokenizer.
///
/// # Arguments
/// * `name` - label for the temp directory `memhub-mcp_<name>_<pid>`.
///
/// # Returns
/// The opened store, shared behind an `Arc`.
///
/// # Errors
/// * Directory creation or database open failures.
pub fn store(name: &str) -> Result<Arc<SqliteStore>> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let path: PathBuf = dir.join("m.db");
    Ok(Arc::new(SqliteStore::open_with_tokenizer(&path, "porter unicode61")?))
}

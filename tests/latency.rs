//! Search latency gate: p95 of hybrid search over 10k chunks. Run with
//! `cargo test --release --test latency -- --ignored`.
//!
//! The limit is the measured exact-tier p95 at 10k chunks (15.7 ms), rounded up to
//! 20 ms. The query embedder here is a fake, so the gate covers keyword + vector + fusion, not
//! model inference.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::{NewChunk, NewNote, SearchFilter, SearchRequest};
use memhub_mcp::error::Result;
use memhub_mcp::ports::embedder::{Embedder, Kind, SharedEmbedder, Tokenizer};
use memhub_mcp::ports::store::{EmbeddingQueue, NoteRecords};

const DIM: usize = 384;
const NOTES: usize = 2000;
const CHUNKS_PER_NOTE: u32 = 5;
const SEARCHES: usize = 1000;
const P95_LIMIT: Duration = Duration::from_millis(20);
const WORDS: [&str; 16] = [
    "rust", "sqlite", "vector", "memory", "search", "daemon", "relay", "token", "chunk", "fusion", "index", "query",
    "model", "note", "tag", "embed",
];

/// Fills a 384-dimensional vector from a xorshift generator seeded by `seed`, then scales it to unit length.
///
/// # Arguments
/// * `seed` - initial generator state; the low bit is forced on so the state never becomes zero.
///
/// # Returns
/// A `DIM`-length vector with L2 norm 1.
fn vector(mut seed: u64) -> Vec<f32> {
    seed |= 1;
    let v: Vec<f32> = (0..DIM)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            f32::from(u16::try_from(seed >> 48).unwrap()) / 65536.0 - 0.5
        })
        .collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.into_iter().map(|x| x / norm).collect()
}

struct HashEmbedder;

impl Embedder for HashEmbedder {
    /// Folds the bytes of `text` into a seed and returns the matching unit vector.
    ///
    /// # Arguments
    /// * `text` - bytes that determine the vector; the `Kind` argument is ignored.
    ///
    /// # Returns
    /// A `DIM`-length unit vector from `vector`.
    ///
    /// # Errors
    /// * Never fails; always returns `Ok`.
    fn embed(&mut self, text: &str, _: Kind) -> Result<Vec<f32>> {
        Ok(vector(text.bytes().fold(7u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)))))
    }
    /// Returns the embedding dimension.
    ///
    /// # Returns
    /// `DIM` (384).
    fn dim(&self) -> usize {
        DIM
    }
    /// Returns the fixed model identity that tags the stored embeddings.
    ///
    /// # Returns
    /// The string `"bench"`.
    fn identity(&self) -> String {
        "bench".into()
    }
    /// Always panics: this embedder is used only for searches, which never chunk text.
    ///
    /// # Panics
    /// Always, via `unreachable!`.
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        unreachable!("searches never chunk")
    }
}

/// Builds a temporary SQLite store seeded with 2000 notes of 5 chunks each, and gives every chunk an embedding.
///
/// Side effects: removes and recreates `memhub-mcp_latency_<pid>` under the system temp dir, opens a
/// `porter unicode61` store there, and writes notes and embeddings to it.
///
/// # Returns
/// The populated store behind an `Arc`.
///
/// # Panics
/// Panics if any filesystem, store, or embedding call fails.
fn seeded_store() -> Arc<SqliteStore> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_latency_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let s = SqliteStore::open_with_tokenizer(&dir.join("m.db"), "porter unicode61").unwrap();
    s.reset_embeddings("bench").unwrap();
    let text = |i: usize| (0..12).map(|k| WORDS[(i * 7 + k * 3) % WORDS.len()]).collect::<Vec<_>>().join(" ");
    for n in 0..NOTES {
        let chunks: Vec<NewChunk> =
            (0..CHUNKS_PER_NOTE).map(|seq| NewChunk { seq, start: 0, end: 1, text: text(n + seq as usize) }).collect();
        s.create(&NewNote { title: format!("note {n}"), body: text(n), tags: vec![] }, &chunks, false).unwrap();
    }
    let mut seed = 1u64;
    while let pending = s.pending_chunks(memhub_mcp::domain::ChunkId(0), 512).unwrap()
        && !pending.is_empty()
    {
        for c in pending {
            seed += 1;
            assert!(s.set_embedding(c.id, &vector(seed)).unwrap());
        }
    }
    Arc::new(s)
}

/// Measures p95 latency of 1000 hybrid searches over the seeded 10k-chunk store and asserts it stays within `P95_LIMIT`.
///
/// Ignored by default; run with `--release -- --ignored`. Runs 50 warmup searches that must take the hybrid path, then times 1000 searches and prints p50 and p95.
///
/// # Panics
/// Panics if a warmup search falls back to keyword-only, if any search errors, or if p95 exceeds `P95_LIMIT`.
#[test]
#[ignore = "benchmark; run with --release -- --ignored"]
fn hybrid_search_p95_within_limit() {
    let embedder: SharedEmbedder = Arc::new(Mutex::new(Box::new(HashEmbedder)));
    let service = SearchService::new(seeded_store(), Some(embedder));
    let request = |i: usize| SearchRequest {
        query: format!("{} {}", WORDS[i % WORDS.len()], WORDS[(i * 5 + 3) % WORDS.len()]),
        limit: 10,
        filter: SearchFilter::default(),
    };
    for i in 0..50 {
        assert!(!service.search(&request(i)).unwrap().keyword_only, "benchmark must take the hybrid path");
    }
    let mut times: Vec<Duration> = (0..SEARCHES)
        .map(|i| {
            let start = Instant::now();
            service.search(&request(i)).unwrap();
            start.elapsed()
        })
        .collect();
    times.sort_unstable();
    let (p50, p95) = (times[SEARCHES / 2], times[SEARCHES * 95 / 100]);
    eprintln!("{} chunks, {SEARCHES} searches: p50 {p50:?}, p95 {p95:?}", NOTES * CHUNKS_PER_NOTE as usize);
    assert!(p95 <= P95_LIMIT, "p95 {p95:?} exceeds {P95_LIMIT:?}");
}

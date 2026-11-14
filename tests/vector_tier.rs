//! Vector tier: exact/turbovec switch, sync with edits and deletes, recall of the indexed tier.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::path::{Path, PathBuf};

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::domain::{ChunkId, NewChunk, NewNote, NoteId, SearchFilter};
use memhub_mcp::ports::store::{EmbeddingQueue, NoteRecords, NoteSearch};

const DIM: usize = 64;

/// Returns `m.db` inside a recreated temp directory named after `name` and the process id.
///
/// Removes any previous directory of that name, then creates the new one on disk.
///
/// # Arguments
/// * `name` - test name used in the directory name.
///
/// # Returns
/// The path to `m.db` in that directory.
///
/// # Panics
/// Panics if the directory cannot be created.
fn db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_tier_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("m.db")
}

/// Opens a `SqliteStore` at `path` with the porter tokenizer and sets the exact-scan threshold to `exact_max` chunks.
///
/// # Arguments
/// * `path` - database file to open.
/// * `exact_max` - chunk count at or below which queries use an exact scan.
///
/// # Returns
/// The configured store.
///
/// # Panics
/// Panics if opening the store or setting the threshold fails.
fn open(path: &Path, exact_max: usize) -> SqliteStore {
    SqliteStore::open_with_tokenizer(path, "porter unicode61").unwrap().with_exact_max_chunks(exact_max).unwrap()
}

/// Seeded xorshift producing unit vectors scattered around a centre.
struct Gen(u64);

impl Gen {
    /// Advances the xorshift state and returns a value in `[-0.5, 0.5)`.
    ///
    /// # Returns
    /// A pseudo-random value from the top 16 bits of the state, scaled by 1/65536 and shifted by 0.5.
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        f32::from(u16::try_from(self.0 >> 48).unwrap()) / 65536.0 - 0.5
    }

    /// Returns a unit vector near `centre`: each component is `centre[i] + spread * next()`, then the vector is normalized.
    ///
    /// # Arguments
    /// * `centre` - centre of the cluster.
    /// * `spread` - scale of the random offset per component.
    ///
    /// # Returns
    /// The normalized vector, with the same length as `centre`.
    fn near(&mut self, centre: &[f32], spread: f32) -> Vec<f32> {
        let v: Vec<f32> = centre.iter().map(|c| c + spread * self.next()).collect();
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / norm).collect()
    }

    /// Returns a random unit vector of length `DIM`, drawn around the origin with spread 1.
    ///
    /// # Returns
    /// A normalized vector of length `DIM`.
    fn random(&mut self) -> Vec<f32> {
        self.near(&[0.0; DIM], 1.0)
    }
}

/// Adds `notes` notes titled `{prefix}{i}`, each with `per_note` chunks, then embeds every pending chunk.
///
/// # Arguments
/// * `s` - store to write into.
/// * `prefix` - title prefix for the created notes.
/// * `notes` - number of notes to create.
/// * `per_note` - chunks per note.
/// * `vector` - supplies the embedding for each pending chunk.
///
/// # Returns
/// The ids of the created notes, in creation order.
///
/// # Panics
/// Panics if a note cannot be created or a chunk embedding cannot be stored.
fn fill(
    s: &SqliteStore,
    prefix: &str,
    notes: usize,
    per_note: u32,
    vector: &mut dyn FnMut() -> Vec<f32>,
) -> Vec<NoteId> {
    let chunks: Vec<NewChunk> = (0..per_note).map(|seq| NewChunk { seq, start: 0, end: 1, text: "x".into() }).collect();
    let ids = (0..notes)
        .map(|i| {
            let note = NewNote { title: format!("{prefix}{i}"), body: "x".into(), tags: vec![] };
            s.create(&note, &chunks, false).unwrap().id
        })
        .collect();
    embed_pending(s, vector);
    ids
}

/// Embeds every pending chunk in batches of 512, calling `vector` once per chunk, until none remain.
///
/// # Arguments
/// * `s` - store whose pending chunks are embedded.
/// * `vector` - supplies an embedding for each chunk.
///
/// # Panics
/// Panics if a batch cannot be read or an embedding is not stored.
fn embed_pending(s: &SqliteStore, vector: &mut dyn FnMut() -> Vec<f32>) {
    loop {
        let pending = s.pending_chunks(memhub_mcp::domain::ChunkId(0), 512).unwrap();
        if pending.is_empty() {
            return;
        }
        for c in pending {
            assert!(s.set_embedding(c.id, &vector()).unwrap());
        }
    }
}

/// Visits every chunk for `query` and returns `(chunk, note)` pairs sorted by descending cosine score.
///
/// # Arguments
/// * `s` - store to visit.
/// * `query` - query vector.
/// * `exact` - `true` for a full scan, `false` for the indexed `for_each_near` path.
///
/// # Returns
/// The visited chunk and note ids, best cosine first.
///
/// # Panics
/// Panics if the scan fails.
fn ranked(s: &SqliteStore, query: &[f32], exact: bool) -> Vec<(ChunkId, NoteId)> {
    let mut out = Vec::new();
    let mut visit = |c, n, v: &[f32]| out.push((v.iter().zip(query).map(|(a, b)| a * b).sum::<f32>(), c, n));
    if exact {
        s.for_each_embedding(&SearchFilter::default(), &mut visit).unwrap();
    } else {
        s.for_each_near(&SearchFilter::default(), query, &mut visit).unwrap();
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.into_iter().map(|(_, c, n)| (c, n)).collect()
}

/// Proves the exact-to-index switch at the 500-chunk threshold in both directions. At 500 chunks every chunk is scanned. Crossing to 550 builds the index and visits 400. A reopened store keeps the index, deleting one note leaves 400, and deleting five more drops to 250 with every chunk scanned again.
#[test]
fn threshold_switches_tiers_both_ways() {
    let path = db("switch");
    let s = open(&path, 500);
    let mut g = Gen(7);
    let notes = fill(&s, "a", 10, 50, &mut || g.random());
    let q = g.random();
    assert_eq!(ranked(&s, &q, false).len(), 500, "at the threshold every chunk is scanned");
    fill(&s, "b", 1, 50, &mut || g.random());
    assert_eq!(ranked(&s, &q, false).len(), 400, "crossing it builds the index");
    drop(s);
    let s = open(&path, 500);
    assert_eq!(ranked(&s, &q, false).len(), 400, "startup builds the index above the threshold");
    s.delete(notes[0]).unwrap();
    assert_eq!(ranked(&s, &q, false).len(), 400, "back at the threshold the index stays");
    for &n in &notes[1..6] {
        s.delete(n).unwrap();
    }
    assert_eq!(ranked(&s, &q, false).len(), 250, "at half the threshold every chunk is scanned again");
}

/// Proves deleted notes and stale chunks never appear in indexed results. The top hit's deleted note is absent afterwards. An edited note is absent until re-embedded, then its new chunk ranks first and the old chunk is gone. A deletion through a second connection is also honored.
#[test]
fn deleted_or_edited_chunks_are_never_returned() {
    let path = db("stale");
    let s = open(&path, 100);
    let mut g = Gen(11);
    fill(&s, "n", 100, 3, &mut || g.random());
    let q = g.random();
    let hits = |s: &SqliteStore| ranked(s, &q, false);

    let (_, deleted) = hits(&s)[0];
    s.delete(deleted).unwrap();
    assert!(hits(&s).iter().all(|&(_, n)| n != deleted), "deleted note returned");

    let (old_chunk, edited) = hits(&s)[0];
    let version = s.read_by_id(edited).unwrap().unwrap().version;
    s.edit(edited, version, "y", &[], &[NewChunk { seq: 0, start: 0, end: 1, text: "y".into() }]).unwrap();
    assert!(hits(&s).iter().all(|&(_, n)| n != edited), "unembedded edit returned");
    embed_pending(&s, &mut || q.clone());
    let after = hits(&s);
    assert_eq!(after[0].1, edited, "re-embedded chunk is indexed");
    assert!(after.iter().all(|&(c, _)| c != old_chunk), "replaced chunk returned");

    let other = SqliteStore::open_with_tokenizer(&path, "porter unicode61").unwrap();
    other.delete(edited).unwrap();
    assert!(hits(&s).iter().all(|&(_, n)| n != edited), "note deleted by another connection returned");
}

/// Proves the indexed tier's recall@10 is at least 0.98 against an exact scan. It builds 100 notes of 50 clustered chunks with the index disabled, reopens with threshold 1000 so the index is used, then compares each of 50 queries' indexed top 10 with the exact top 10.
#[test]
fn indexed_recall_at_10_meets_floor() {
    let path = db("recall");
    let mut g = Gen(42);
    let centres: Vec<Vec<f32>> = (0..50).map(|_| g.random()).collect();
    let mut i = 0;
    fill(&open(&path, usize::MAX), "n", 100, 50, &mut || {
        i += 1;
        g.near(&centres[i % centres.len()], 0.6)
    });
    let s = open(&path, 1000);
    let (mut found, queries) = (0, 50);
    for qi in 0..queries {
        let q = g.near(&centres[qi % centres.len()], 0.6);
        let exact: Vec<_> = ranked(&s, &q, true).into_iter().take(10).collect();
        let near: Vec<_> = ranked(&s, &q, false).into_iter().take(10).collect();
        found += exact.iter().filter(|h| near.contains(h)).count();
    }
    let recall = f64::from(u32::try_from(found).unwrap()) / f64::from(u32::try_from(queries * 10).unwrap());
    assert!(recall >= 0.98, "recall@10 {recall}");
}

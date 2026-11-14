//! Edits re-embed only new or changed chunks; tags-only edits keep every vector.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::domain::{EditOp, NewChunk, NewNote, NoteId, SearchFilter, Tag};
use memhub_mcp::ports::embedder::{Kind, SharedEmbedder};
use memhub_mcp::ports::store::{EmbeddingQueue, NoteRecords, NoteSearch, NoteStore};

/// Opens a fresh SQLite store in a wiped per-test temp dir with the Porter tokenizer and the given
/// exact-search threshold.
///
/// # Arguments
/// * `name` - Test-specific directory suffix.
/// * `exact_max` - Maximum embedded chunk count served by the exact (brute-force) vector scan;
///   above it the fast approximate tier is used. `usize::MAX` forces exact.
///
/// # Returns
/// The shared store.
///
/// # Panics
/// Panics if the directory or store cannot be created.
fn open(name: &str, exact_max: usize) -> Arc<SqliteStore> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_incr_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path: PathBuf = dir.join("m.db");
    Arc::new(
        SqliteStore::open_with_tokenizer(&path, "porter unicode61").unwrap().with_exact_max_chunks(exact_max).unwrap(),
    )
}

/// Embeds every pending chunk with `embedder` as documents, 64 at a time, until none remain.
///
/// # Arguments
/// * `store` - Store whose pending chunks are embedded and saved.
/// * `embedder` - Shared embedder, locked per chunk.
///
/// # Returns
/// Number of chunks embedded.
///
/// # Panics
/// Panics on any store/embedder error or if a chunk's embedding is not saved.
fn drain(store: &SqliteStore, embedder: &SharedEmbedder) -> usize {
    let mut n = 0;
    loop {
        let batch = store.pending_chunks(memhub_mcp::domain::ChunkId(0), 64).unwrap();
        if batch.is_empty() {
            return n;
        }
        for c in batch {
            let v = embedder.lock().unwrap().embed(&c.text, Kind::Document).unwrap();
            assert!(store.set_embedding(c.id, &v).unwrap());
            n += 1;
        }
    }
}

/// Builds a [`NewNote`] tagged "a".
///
/// # Arguments
/// * `title` - Note title.
/// * `body` - Note body.
///
/// # Returns
/// The note to create.
fn note(title: &str, body: &str) -> NewNote {
    NewNote { title: title.into(), body: body.into(), tags: vec![Tag("a".into())] }
}

/// Builds distinct filler words "w{i} " for each `i` in `range` (one token each).
///
/// # Arguments
/// * `range` - Word indices.
///
/// # Returns
/// The concatenated words, each followed by a space.
fn words(range: std::ops::Range<usize>) -> String {
    range.map(|i| format!("w{i} ")).collect()
}

/// Reports whether a vector search for `query` reaches note `id` among the store's embedded chunks.
///
/// # Arguments
/// * `store` - Store to scan.
/// * `embedder` - Embedder for the query vector.
/// * `query` - Query text.
/// * `id` - Note expected in the results.
///
/// # Returns
/// `true` if any visited chunk belongs to `id`.
///
/// # Panics
/// Panics on embedder or store errors.
fn found(store: &SqliteStore, embedder: &SharedEmbedder, query: &str, id: NoteId) -> bool {
    let q = embedder.lock().unwrap().embed(query, Kind::Query).unwrap();
    let mut hit = false;
    store.for_each_near(&SearchFilter::default(), &q, &mut |_, n, _| hit |= n == id).unwrap();
    hit
}

/// Proves appending to a three-chunk note re-embeds only the changed last chunk: with an 8-token
/// window, 20 words make 3 chunks, and after appending one word 1 chunk is pending and 3 exist.
#[test]
fn append_to_three_chunk_note_queues_one_chunk() {
    let store = open("append3", usize::MAX);
    let (embedder, _) = common::embedder("m", 8);
    let svc = NoteService::new(Arc::clone(&store) as Arc<dyn NoteStore>, Some(Arc::clone(&embedder)), None);
    // Window 8, stride 7: 20 words make chunks at 0, 7, 14.
    let n = svc.write(note("t", &words(0..20)), false).unwrap();
    assert_eq!(drain(&store, &embedder), 3);
    let edited = svc.edit(n.id, n.version, EditOp::Append("x1 ".into())).unwrap();
    assert_eq!(store.pending_count().unwrap(), 1, "only the last chunk changed");
    assert_eq!(store.chunk_count().unwrap(), 3);
    assert_eq!(edited.body, format!("{}x1 ", words(0..20)));
}

/// Proves appending to a ~1 MB note (over 1000 chunks) queues at most 2 chunks for re-embedding.
#[test]
fn append_to_big_note_queues_constant_chunks() {
    let store = open("append_big", usize::MAX);
    let (embedder, _) = common::embedder("m", 64);
    let svc = NoteService::new(Arc::clone(&store) as Arc<dyn NoteStore>, Some(Arc::clone(&embedder)), None);
    let mut body = String::new();
    let mut i = 0;
    while body.len() < 1_040_000 {
        body.push_str(&format!("w{i} "));
        i += 1;
    }
    let n = svc.write(note("big", &body), false).unwrap();
    let total = store.pending_count().unwrap();
    assert!(total > 1000, "{total}");
    // Cheap fixed vectors: this test counts queued chunks, not search quality.
    for c in store.pending_chunks(memhub_mcp::domain::ChunkId(0), total).unwrap() {
        store.set_embedding(c.id, &[1.0, 0.0]).unwrap();
    }
    svc.edit(n.id, n.version, EditOp::Append("tail words here ".into())).unwrap();
    let queued = store.pending_count().unwrap();
    assert!(queued <= 2, "append queued {queued} of {total} chunks");
}

/// Shared body for the tags-only edit tests: in a four-note store, changing one note's tags, and then
/// replacing its body with identical text, queues no chunk, keeps the chunk count and keeps the note
/// findable by vector search.
///
/// # Arguments
/// * `exact_max` - Exact vector-scan threshold for the store (selects exact or fast tier).
/// * `name` - Temp directory suffix.
///
/// # Panics
/// Panics if any assertion or store call fails.
fn tag_edit_keeps_vectors(exact_max: usize, name: &str) {
    let store = open(name, exact_max);
    let (embedder, _) = common::embedder("m", 8);
    let svc = NoteService::new(Arc::clone(&store) as Arc<dyn NoteStore>, Some(Arc::clone(&embedder)), None);
    let mut ids = Vec::new();
    for k in 0..4 {
        let body = format!("{} zebra{k} stripes{k}", words(k * 30..k * 30 + 20));
        ids.push(svc.write(note(&format!("n{k}"), &body), false).unwrap());
    }
    drain(&store, &embedder);
    let chunks = store.chunk_count().unwrap();
    let target = &ids[2];
    assert!(found(&store, &embedder, "zebra2 stripes2", target.id));
    let edited = svc.edit(target.id, target.version, EditOp::SetTags(vec![Tag("b".into())])).unwrap();
    assert_eq!(edited.tags, vec![Tag("b".into())]);
    assert_eq!(store.pending_count().unwrap(), 0, "tags-only edit queues nothing");
    assert_eq!(store.chunk_count().unwrap(), chunks);
    assert!(found(&store, &embedder, "zebra2 stripes2", target.id), "exact_max {exact_max}");
    // Same body via replace: still nothing to re-embed or forget.
    let again = svc.edit(target.id, edited.version, EditOp::ReplaceBody(edited.body.clone())).unwrap();
    assert_eq!(again.version, edited.version + 1);
    assert_eq!(store.pending_count().unwrap(), 0);
    assert!(found(&store, &embedder, "zebra2 stripes2", target.id), "exact_max {exact_max}");
}

/// Proves tags-only and same-body edits keep vectors when searches use the exact vector scan.
#[test]
fn tags_only_edit_keeps_vectors_exact_tier() {
    tag_edit_keeps_vectors(usize::MAX, "tags_exact");
}

/// Proves tags-only and same-body edits keep vectors when the fast approximate vector tier is used
/// (exact threshold 2 chunks).
#[test]
fn tags_only_edit_keeps_vectors_fast_tier() {
    tag_edit_keeps_vectors(2, "tags_fast");
}

/// Proves that when an edit keeps one of two chunks with identical text, the embedded row is kept:
/// after embedding the second "dup" chunk and editing to a single "dup" chunk, 1 chunk exists and 0 are pending.
#[test]
fn duplicate_text_keeps_the_embedded_row() {
    let store = open("dup", usize::MAX);
    let c = |seq: u32, start: usize| NewChunk { seq, start, end: start + 3, text: "dup".into() };
    let n = store.create(&note("d", "dup dup"), &[c(0, 0), c(1, 4)], false).unwrap();
    let pending = store.pending_chunks(memhub_mcp::domain::ChunkId(0), 8).unwrap();
    store.set_embedding(pending[1].id, &[1.0, 0.0]).unwrap();
    store.edit(n.id, n.version, "dup", &[], &[c(0, 0)]).unwrap();
    assert_eq!((store.pending_count().unwrap(), store.chunk_count().unwrap()), (0, 1));
}

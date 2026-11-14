//! Integration tests for the SQLite `NoteStore`.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::path::PathBuf;
use std::sync::Arc;

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::{ListQuery, NewChunk, NewNote, NoteId, SearchFilter, SearchRequest, Tag};
use memhub_mcp::error::Error;
use memhub_mcp::ports::store::{EmbeddingQueue, NoteRecords, NoteSearch};

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
    let dir = std::env::temp_dir().join(format!("memhub-mcp_test_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("m.db")
}

/// Builds a `NewNote` with the given title, body, and tags converted to `Tag`s.
fn note(title: &str, body: &str, tags: &[&str]) -> NewNote {
    NewNote { title: title.into(), body: body.into(), tags: tags.iter().map(|t| Tag((*t).into())).collect() }
}

/// Builds a `NewChunk` covering all of `text`: seq 0, start 0, end equal to the byte length.
fn chunk(text: &str) -> NewChunk {
    NewChunk { seq: 0, start: 0, end: text.len(), text: text.into() }
}

/// Proves create, read, conflict, overwrite, and edit versioning. A title conflict returns `Conflict` with the existing id, `overwrite` bumps the version to 2, a stale edit returns `VersionMismatch`, and editing an unknown id returns `NotFound`.
#[test]
fn crud_conflict_and_version() {
    let s = SqliteStore::open_with_tokenizer(&db("crud"), "porter unicode61").unwrap();
    let n = s.create(&note("Alpha", "first", &["b", "a"]), &[], false).unwrap();
    assert_eq!((n.version, n.tags.len()), (1, 2));
    assert_eq!(s.read_by_title("ALPHA").unwrap().unwrap().id, n.id);
    match s.create(&note("alpha", "x", &[]), &[], false) {
        Err(Error::Conflict { existing_id }) => assert_eq!(existing_id, n.id),
        other => panic!("{other:?}"),
    }
    let o = s.create(&note("alpha", "second", &[]), &[], true).unwrap();
    assert_eq!((o.id, o.version, o.body.as_str(), o.title.as_str()), (n.id, 2, "second", "alpha"));
    let e = s.edit(n.id, 2, "third", &[Tag("t".into())], &[]).unwrap();
    assert_eq!(e.version, 3);
    assert!(matches!(s.edit(n.id, 2, "x", &[], &[]), Err(Error::VersionMismatch { expected: 2, actual: 3 })));
    assert!(matches!(s.edit(NoteId(999), 1, "x", &[], &[]), Err(Error::NotFound)));
}

/// Proves a soft-deleted note disappears from reads, lists, keyword search, pending chunks, and embedding scans. Deleting it again returns `NotFound`, and its title can be reused.
#[test]
fn soft_delete_hidden_everywhere() {
    let s = SqliteStore::open_with_tokenizer(&db("delete"), "porter unicode61").unwrap();
    let n = s.create(&note("Gone", "zebra stripes", &["t"]), &[chunk("zebra")], false).unwrap();
    let pending = s.pending_chunks(memhub_mcp::domain::ChunkId(0), 10).unwrap();
    assert!(s.set_embedding(pending[0].id, &[1.0, 2.0]).unwrap());
    s.delete(n.id).unwrap();
    assert!(s.read_by_id(n.id).unwrap().is_none());
    assert!(s.read_by_title("gone").unwrap().is_none());
    assert!(s.list(&ListQuery { limit: 10, ..Default::default() }).unwrap().items.is_empty());
    assert!(s.fts_search("zebra", &SearchFilter::default(), 10).unwrap().is_empty());
    assert!(s.pending_chunks(memhub_mcp::domain::ChunkId(0), 10).unwrap().is_empty());
    let mut seen = 0;
    s.for_each_embedding(&Default::default(), &mut |_, _, _| seen += 1).unwrap();
    assert_eq!(seen, 0);
    assert!(matches!(s.delete(n.id), Err(Error::NotFound)));
    s.create(&note("Gone", "again", &[]), &[], false).unwrap();
}

/// Proves cursor pagination over seven notes with limit 3 yields three pages in descending id order. A tag filter returns the four `even` notes, and a malformed cursor is rejected with `InvalidInput`.
#[test]
fn pagination_three_pages() {
    let s = SqliteStore::open_with_tokenizer(&db("pages"), "porter unicode61").unwrap();
    for i in 0..7 {
        let tags: &[&str] = if i % 2 == 0 { &["even"] } else { &[] };
        s.create(&note(&format!("n{i}"), "b", tags), &[], false).unwrap();
    }
    let mut q = ListQuery { limit: 3, ..Default::default() };
    let mut ids = Vec::new();
    let mut pages = 0;
    loop {
        let p = s.list(&q).unwrap();
        pages += 1;
        ids.extend(p.items.iter().map(|n| n.id.0));
        match p.next_cursor {
            Some(c) => q.cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(ids, (1..=7).rev().collect::<Vec<_>>());
    let even = ListQuery { limit: 10, tag: Some(Tag("even".into())), ..Default::default() };
    assert_eq!(s.list(&even).unwrap().items.len(), 4);
    let bad = ListQuery { limit: 1, cursor: Some(memhub_mcp::domain::Cursor("x".into())), ..Default::default() };
    assert!(matches!(s.list(&bad), Err(Error::InvalidInput(_))));
}

/// Proves FTS ranks the note with more `rust` occurrences first, returns the `[rust]`-marked snippet for it, and returns an empty snippet for a non-matching note.
#[test]
fn fts_ranks_best_first() {
    let s = SqliteStore::open_with_tokenizer(&db("fts"), "porter unicode61").unwrap();
    s.create(&note("Other", "a note about cats and some rust", &[]), &[], false).unwrap();
    let best = s.create(&note("Rust tips", "rust rust borrowing in rust", &[]), &[], false).unwrap();
    let hits = s.fts_search("\"rust\"", &SearchFilter::default(), 10).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].0, best.id);
    assert!(hits[0].1 < hits[1].1);
    assert_eq!(s.fts_snippet("\"rust\"", best.id).unwrap(), "[rust] [rust] borrowing in [rust]");
    assert_eq!(s.fts_snippet("\"cats\"", best.id).unwrap(), "", "non-matching note has no snippet");
}

/// Proves FTS snippets reach hits far into a large note and stemmed matches. A `needle` after 20,000 filler words must appear as `[needle]`, and `run` must match `running` as a stemmed-only hit.
#[test]
fn large_note_snippets_cover_far_and_stemmed_hits() {
    let s = SqliteStore::open_with_tokenizer(&db("bigsnip"), "porter unicode61").unwrap();
    let body = format!("{}needle here\n{}", "filler ".repeat(20_000), "running fast");
    let n = s.create(&note("Big", &body, &[]), &[], false).unwrap();
    assert!(s.fts_snippet("\"needle\"", n.id).unwrap().contains("[needle]"));
    assert!(s.fts_snippet("\"run\"", n.id).unwrap().contains("[running]"), "stemmed-only hit");
}

/// Benchmarks keyword search over 20 notes of about 1 MiB each with a 32-term query. Asserts three hits with `[term` snippets and a runtime under one second.
///
/// Ignored by default because it is wall-clock bound; run with `--release -- --ignored`.
#[test]
#[ignore = "wall-clock bound; run with --release -- --ignored"]
fn keyword_search_over_many_large_notes_is_fast() {
    let s = Arc::new(SqliteStore::open_with_tokenizer(&db("large"), "porter unicode61").unwrap());
    let words: Vec<String> = (0..32).map(|i| format!("term{i}")).collect();
    let para = format!("{}\n\n", words.join(" "));
    let body: String = para.repeat(memhub_mcp::domain::MAX_BODY / para.len());
    for i in 0..20 {
        s.create(&note(&format!("big{i}"), &body, &[]), &[], false).unwrap();
    }
    let search = SearchService::new(s, None);
    let req = SearchRequest { query: words.join(" "), limit: 3, filter: SearchFilter::default() };
    let start = std::time::Instant::now();
    let res = search.search(&req).unwrap();
    assert_eq!(res.hits.len(), 3);
    assert!(res.hits.iter().all(|h| h.snippet.contains("[term")));
    let took = start.elapsed();
    println!("20 x 1 MiB notes, 32-term keyword search: {took:?}");
    assert!(took < std::time::Duration::from_secs(1), "took {took:?}");
}

/// Proves `set_embedding` ignores stale chunk ids and rejects invalid vectors. After an edit, the old chunk id returns false, the new id accepts `[0.5, -1.0]` once, and a second write returns false. Zero, NaN, and infinite vectors error, and only the current embedding is listed.
#[test]
fn stale_set_embedding_is_noop() {
    let s = SqliteStore::open_with_tokenizer(&db("stale"), "porter unicode61").unwrap();
    let n = s.create(&note("E", "one", &[]), &[chunk("one")], false).unwrap();
    let old = s.pending_chunks(memhub_mcp::domain::ChunkId(0), 10).unwrap()[0].id;
    s.edit(n.id, 1, "two", &[], &[chunk("two")]).unwrap();
    assert!(!s.set_embedding(old, &[0.5]).unwrap());
    let new = s.pending_chunks(memhub_mcp::domain::ChunkId(0), 10).unwrap()[0].id;
    assert!(s.set_embedding(new, &[0.5, -1.0]).unwrap());
    assert!(!s.set_embedding(new, &[1.0, 0.0]).unwrap());
    for bad in [[0.0, 0.0], [f32::NAN, 1.0], [f32::INFINITY, 0.0]] {
        assert!(s.set_embedding(new, &bad).is_err(), "{bad:?}");
    }
    let mut got = Vec::new();
    s.for_each_embedding(&Default::default(), &mut |c, id, v| got.push((c, id, v.to_vec()))).unwrap();
    assert_eq!(got, vec![(new, n.id, vec![0.5, -1.0])]);
}

/// Proves a tags-only edit keeps a note's chunk embedding. After the chunk is embedded, the tag edit leaves nothing pending, and the embedding scan still yields the original chunk id.
#[test]
fn tags_only_edit_keeps_embeddings() {
    let s = SqliteStore::open_with_tokenizer(&db("tags_only"), "porter unicode61").unwrap();
    let n = s.create(&note("T", "one", &[]), &[chunk("one")], false).unwrap();
    let id = s.pending_chunks(memhub_mcp::domain::ChunkId(0), 10).unwrap()[0].id;
    assert!(s.set_embedding(id, &[0.5]).unwrap());
    s.edit(n.id, 1, "one", &[Tag("k".into())], &[]).unwrap();
    assert!(s.pending_chunks(memhub_mcp::domain::ChunkId(0), 10).unwrap().is_empty());
    let mut got = Vec::new();
    s.for_each_embedding(&Default::default(), &mut |c, _, _| got.push(c)).unwrap();
    assert_eq!(got, vec![id]);
}

/// Proves two store handles on one database file can write concurrently without losing rows. Eight threads (four per handle) each create 25 notes, and a raw connection then counts exactly 200 notes.
#[test]
fn two_stores_write_concurrently() {
    let path = db("concurrent");
    let a = Arc::new(SqliteStore::open_with_tokenizer(&path, "porter unicode61").unwrap());
    let b = Arc::new(SqliteStore::open_with_tokenizer(&path, "porter unicode61").unwrap());
    let handles: Vec<_> = [a, b]
        .into_iter()
        .enumerate()
        .flat_map(|(si, s)| {
            (0..4).map(move |t| {
                let s = Arc::clone(&s);
                std::thread::spawn(move || {
                    for i in 0..25 {
                        s.create(&note(&format!("s{si}t{t}i{i}"), "body", &["x"]), &[chunk("body")], false)?;
                    }
                    Ok::<_, Error>(())
                })
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap().unwrap();
    }
    drop(SqliteStore::open_with_tokenizer(&path, "porter unicode61").unwrap());
    let raw = rusqlite::Connection::open(&path).unwrap();
    let n: i64 = raw.query_row("SELECT count(*) FROM notes", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 200);
}

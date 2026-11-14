//! Regression tests for audited behaviour: pragmas, ids, titles, MCP errors and hybrid ranking.

mod common;

use std::sync::{Arc, Mutex};

use memhub_mcp::app::embed_worker;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::app::reembed::ReembedService;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::{NewNote, NoteId, SearchFilter, SearchHit, SearchRequest};
use memhub_mcp::error::{Error, Result};
use memhub_mcp::mcp::server::MemoryServer;
use memhub_mcp::ports::embedder::{Embedder, Kind, SharedEmbedder, Tokenizer};
use memhub_mcp::ports::store::NoteStore;
use rmcp::ServiceExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Builds a `NewNote` with the given title and body and no tags.
fn note(title: &str, body: &str) -> NewNote {
    NewNote { title: title.into(), body: body.into(), tags: vec![] }
}

/// Returns the per-process test database path `<temp dir>/memhub-mcp_<name>_<pid>/m.db`.
///
/// Pure: computes the path only and creates nothing on disk.
///
/// # Arguments
/// * `name` - test name that keeps concurrent test databases apart.
///
/// # Returns
/// The path to `m.db` inside that test's temp directory.
fn db_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("memhub-mcp_{name}_{}", std::process::id())).join("m.db")
}

/// Proves the store opens in WAL mode and that a write waits out a briefly held lock. A second connection takes `BEGIN IMMEDIATE` and commits after 200 ms; the store's write must succeed instead of failing with SQLITE_BUSY.
#[test]
fn store_uses_wal_and_waits_for_locks() -> Result<()> {
    let store = common::store("pragmas")?;
    let other = rusqlite::Connection::open(db_path("pragmas")).map_err(|e| Error::Storage(e.to_string()))?;
    let mode: String =
        other.query_row("PRAGMA journal_mode", [], |r| r.get(0)).map_err(|e| Error::Storage(e.to_string()))?;
    assert_eq!(mode, "wal");
    // Another writer holds the lock briefly; with busy_timeout=0 the store's write fails with SQLITE_BUSY.
    other.execute_batch("BEGIN IMMEDIATE").map_err(|e| Error::Storage(e.to_string()))?;
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        other.execute_batch("COMMIT").is_ok()
    });
    let svc = NoteService::new(store, None, None);
    assert!(svc.write(note("a", "b"), false).is_ok(), "write must wait for the lock");
    assert!(release.join().unwrap_or(false));
    Ok(())
}

/// Proves title uniqueness ignores case: writing `Foo` after `foo` returns `Conflict` carrying the original note's id.
#[test]
fn title_conflict_is_case_insensitive() -> Result<()> {
    let svc = NoteService::new(common::store("conflict")?, None, None);
    let first = svc.write(note("foo", "x"), false)?;
    assert!(
        matches!(svc.write(note("Foo", "y"), false), Err(Error::Conflict { existing_id }) if existing_id == first.id)
    );
    Ok(())
}

/// Proves ids are never reused: a note written after a delete gets an id greater than the deleted note's.
#[test]
fn deleted_ids_are_not_reused() -> Result<()> {
    let svc = NoteService::new(common::store("ids")?, None, None);
    let first = svc.write(note("foo", "x"), false)?;
    svc.delete(first.id)?;
    let second = svc.write(note("bar", "z"), false)?;
    assert!(second.id.0 > first.id.0, "id {} reused", second.id.0);
    Ok(())
}

/// Proves a failing tool call (`read_note` for missing id 99) returns a JSON-RPC result with `isError: true` rather than a transport error. Drives `MemoryServer` over an in-memory duplex through a real `initialize` handshake.
#[tokio::test(flavor = "multi_thread")]
async fn failing_tool_call_returns_is_error() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("mcp_error")?;
    let notes = Arc::new(NoteService::new(Arc::clone(&store), None, None));
    let search = Arc::new(SearchService::new(Arc::clone(&store), None));
    let server = MemoryServer::new(notes, search, Arc::new(ReembedService::new(store, None, None)));
    let (client, srv) = tokio::io::duplex(1 << 16);
    let (sr, sw) = tokio::io::split(srv);
    tokio::spawn(async move {
        if let Ok(running) = server.serve((sr, sw)).await {
            let _ = running.waiting().await;
        }
    });
    let (cr, mut cw) = tokio::io::split(client);
    let mut lines = BufReader::new(cr).lines();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_note","arguments":{"id":99}}}"#;
    cw.write_all(format!("{init}\n").as_bytes()).await?;
    assert!(lines.next_line().await?.is_some_and(|l| l.contains("\"id\":1")));
    cw.write_all(format!("{initialized}\n{call}\n").as_bytes()).await?;
    let reply = lines.next_line().await?.unwrap_or_default();
    let v: rmcp::serde_json::Value =
        rmcp::serde_json::from_str(&reply).map_err(|e| Error::InvalidInput(e.to_string()))?;
    assert_eq!(v["id"], 2);
    assert_eq!(v["result"]["isError"], true, "{reply}");
    Ok(())
}

/// Every text embeds to the same unit vector, so the vector list is all ties.
struct FlatEmbedder;

impl Embedder for FlatEmbedder {
    /// Returns the same unit-length vector for every input, so all vector scores tie.
    ///
    /// # Arguments
    /// * `_` - text to embed; ignored.
    /// * `_` - embedding kind; ignored.
    ///
    /// # Returns
    /// The vector `[0.5, 0.5, 0.5, 0.5]`.
    ///
    /// # Errors
    /// * never; always returns `Ok`.
    fn embed(&mut self, _: &str, _: Kind) -> Result<Vec<f32>> {
        Ok(vec![0.5; 4])
    }
    /// Returns the embedding dimension, 4.
    fn dim(&self) -> usize {
        4
    }
    /// Returns the fixed model identity `flat`.
    fn identity(&self) -> String {
        "flat".into()
    }
    /// Returns a `FakeTokenizer` with a 64-token limit.
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        Arc::new(common::FakeTokenizer { max_tokens: 64 })
    }
}

/// Writes `notes` into a fresh store under `name`, embeds them with `e`, and runs `query` through hybrid search.
///
/// # Arguments
/// * `name` - store name passed to `common::store`.
/// * `e` - shared embedder used for writes and search.
/// * `notes` - notes to write, in order.
/// * `query` - search text.
///
/// # Returns
/// The hits from the hybrid search.
///
/// # Errors
/// * store creation, writes, embedding drain, or search fails.
///
/// # Panics
/// Panics via `assert!` if the search is keyword-only.
fn hybrid(name: &str, e: &SharedEmbedder, notes: &[NewNote], query: &str) -> Result<Vec<SearchHit>> {
    let store: Arc<dyn NoteStore> = common::store(name)?;
    embed_worker::adopt_if_fresh(store.as_ref(), e)?;
    let svc = NoteService::new(Arc::clone(&store), Some(Arc::clone(e)), None);
    for n in notes {
        svc.write(n.clone(), false)?;
    }
    embed_worker::drain(store.as_ref(), e, &mut |_| {})?;
    let req = SearchRequest { query: query.into(), limit: 10, filter: SearchFilter::default() };
    let res = SearchService::new(store, Some(Arc::clone(e))).search(&req)?;
    assert!(!res.keyword_only);
    Ok(res.hits)
}

/// Proves keyword ranking breaks ties when every vector score is equal. With a flat embedder, note 2 (three `rust` occurrences) must rank above note 1, so only the BM25 sign can order them.
#[test]
fn flat_vectors_rank_best_keyword_match_first() -> Result<()> {
    let e: SharedEmbedder = Arc::new(Mutex::new(Box::new(FlatEmbedder)));
    // Note 2 matches best; vector ties break toward note 1, so only the bm25 sign can put note 2 first.
    let notes = [note("one", "rust and many other unrelated filler words here"), note("two", "rust rust rust")];
    let ids: Vec<NoteId> = hybrid("flat", &e, &notes, "rust")?.into_iter().map(|h| h.note_id).collect();
    assert_eq!(ids, vec![NoteId(2), NoteId(1)]);
    Ok(())
}

/// Proves a note with several matching chunks yields one hit. Note `a` has four `apple` chunks and note `b` has one, so exactly two hits come back.
#[test]
fn one_hit_per_note_when_many_chunks_match() -> Result<()> {
    let (e, _) = common::embedder("m", 4);
    let body = "apple pie one\n\napple pie two\n\napple pie three\n\napple pie four";
    let hits = hybrid("chunks", &e, &[note("a", body), note("b", "apple tart")], "apple")?;
    assert_eq!(hits.len(), 2, "{:?}", hits.iter().map(|h| h.note_id).collect::<Vec<_>>());
    Ok(())
}

/// Proves a hit found only by the vector side shows its best chunk. The query `aple` has no keyword match, and the snippet must contain `apple`, the chunk the vector side chose.
#[test]
fn vector_only_hit_shows_its_best_chunk() -> Result<()> {
    let (e, _) = common::embedder("m", 4);
    // "aple" has no keyword match, so the snippet is the vector side's chosen chunk.
    let body = "banana bread here now\n\napple tart here now\n\ncherry cake here now";
    let hits = hybrid("best_chunk", &e, &[note("a", body)], "aple")?;
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.contains("apple"), "{}", hits[0].snippet);
    Ok(())
}

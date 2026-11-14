//! Milestone 2: chunking, pending embeddings, model identity and hybrid fusion.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use memhub_mcp::app::embed_worker;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::{NewNote, SearchFilter, SearchRequest, SearchResults};
use memhub_mcp::error::Result;
use memhub_mcp::ports::embedder::SharedEmbedder;
use memhub_mcp::ports::store::NoteStore;

/// Adopts the embedder's model identity into `store` if the store is fresh, then builds a `NoteService` and a `SearchService` sharing `store` and `e`.
///
/// # Arguments
/// * `store` - note store shared by both services.
/// * `e` - embedder shared by both services.
///
/// # Returns
/// The `(NoteService, SearchService)` pair.
///
/// # Errors
/// * propagates failures from `embed_worker::adopt_if_fresh`.
fn services(store: &Arc<dyn NoteStore>, e: &SharedEmbedder) -> Result<(NoteService, SearchService)> {
    embed_worker::adopt_if_fresh(store.as_ref(), e)?;
    Ok((
        NoteService::new(Arc::clone(store), Some(Arc::clone(e)), None),
        SearchService::new(Arc::clone(store), Some(Arc::clone(e))),
    ))
}

/// Builds a `NewNote` with the given title and body and no tags.
fn note(title: &str, body: &str) -> NewNote {
    NewNote { title: title.into(), body: body.into(), tags: vec![] }
}

/// Runs `query` through `svc` with limit 10 and no filter.
///
/// # Arguments
/// * `svc` - search service to query.
/// * `query` - search text.
///
/// # Returns
/// The search results.
///
/// # Errors
/// * propagates search failures.
fn search(svc: &SearchService, query: &str) -> Result<SearchResults> {
    svc.search(&SearchRequest { query: query.into(), limit: 10, filter: SearchFilter::default() })
}

/// Proves a pending, unembedded chunk does not switch off vector search. Note `a` is embedded, then note `b` is written while the embedder is down. The search must stay hybrid, report one pending embedding, and still find `b` by keyword. After a drain, pending drops to 0.
#[test]
fn pending_chunks_do_not_disable_vector_search() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("pending")?;
    let (e, down) = common::embedder("m1", 64);
    let (notes, searcher) = services(&store, &e)?;
    notes.write(note("a", "apple pie recipe"), false)?;
    assert_eq!(embed_worker::drain(store.as_ref(), &e, &mut |_| {})?, 1);
    down.store(true, Ordering::SeqCst);
    notes.write(note("b", "banana bread recipe"), false)?;
    down.store(false, Ordering::SeqCst);
    let res = search(&searcher, "banana")?;
    assert!(!res.keyword_only, "one pending chunk must not turn off vector search");
    assert_eq!(res.pending_embeddings, 1);
    assert!(!res.hits.is_empty(), "pending note is still found by keyword");
    assert_eq!(embed_worker::drain(store.as_ref(), &e, &mut |_| {})?, 1);
    assert_eq!(search(&searcher, "banana")?.pending_embeddings, 0);
    Ok(())
}

/// Proves a model switch makes search keyword-only until `reembed` rebuilds the vectors. Two notes are written under model `old`. Under `new`, search is keyword-only and draining embeds nothing. `reembed` then processes both notes, records model identity `new`, and search becomes hybrid.
#[test]
fn model_mismatch_is_keyword_only_until_reembed() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("mismatch")?;
    let (old, _) = common::embedder("old", 64);
    let (notes, _) = services(&store, &old)?;
    notes.write(note("a", "apple pie"), false)?;
    notes.write(note("b", "banana bread"), false)?;
    let (new, _) = common::embedder("new", 64);
    let (_, searcher) = services(&store, &new)?;
    assert!(search(&searcher, "apple")?.keyword_only);
    assert_eq!(embed_worker::drain(store.as_ref(), &new, &mut |_| {})?, 0, "mismatch never embeds");
    let mut last = (0, 0);
    let done = embed_worker::reembed(store.as_ref(), &new, &mut |d, t| last = (d, t))?;
    assert_eq!((done, last), (2, (2, 2)));
    assert_eq!(store.model_identity()?.as_deref(), Some("new"));
    assert!(!search(&searcher, "apple")?.keyword_only);
    Ok(())
}

/// Proves a fact buried deep in a long note is found through its chunk. A roughly 1500-word note ends with a zebra-migration sentence, next to a short note. The long note must split into more than 10 chunks, and a misspelled query must rank it first with a snippet containing `zebra migration`.
#[test]
fn late_fact_in_long_note_is_found_through_its_chunk() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("late")?;
    let (e, _) = common::embedder("m1", 128);
    let (notes, searcher) = services(&store, &e)?;
    let mut body: String = (0..1500).map(|i| format!("lorem{} ", i % 50)).collect();
    body.push_str("the zebra migration happens in winter");
    let long = notes.write(note("long", &body), false)?;
    notes.write(note("short", "cats sleep a lot"), false)?;
    let chunks = store.pending_count()? + {
        let mut n = 0;
        store.for_each_embedding(&SearchFilter::default(), &mut |_, _, _| n += 1)?;
        n
    };
    assert!(chunks > 10, "long note is split, got {chunks}");
    embed_worker::drain(store.as_ref(), &e, &mut |_| {})?;
    let res = search(&searcher, "zebrra migrashun")?;
    assert!(!res.keyword_only);
    assert_eq!(res.hits[0].note_id, long.id);
    assert!(res.hits[0].snippet.contains("zebra migration"));
    Ok(())
}

/// Proves keyword and vector lists fuse by rank for query `apple`. Note `two` (apple only) ranks first, note `one` (apple banana) second, and note `three` (vector only, snippet `cherry`) third, with ranks 1, 2, 3.
#[test]
fn hybrid_fuses_keyword_and_vector_lists() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("fusion")?;
    let (e, _) = common::embedder("m1", 64);
    let (notes, searcher) = services(&store, &e)?;
    let both = notes.write(note("one", "apple banana"), false)?;
    let best = notes.write(note("two", "apple"), false)?;
    let vector_only = notes.write(note("three", "cherry"), false)?;
    embed_worker::drain(store.as_ref(), &e, &mut |_| {})?;
    let res = search(&searcher, "apple")?;
    assert!(!res.keyword_only);
    let ids: Vec<_> = res.hits.iter().map(|h| h.note_id).collect();
    assert_eq!(ids, vec![best.id, both.id, vector_only.id]);
    assert_eq!(res.hits.iter().map(|h| h.rank).collect::<Vec<_>>(), vec![1.0, 2.0, 3.0]);
    assert_eq!(res.hits[2].snippet, "cherry");
    Ok(())
}

/// Proves a write commits chunks without embedding them and wakes the drainer. A counting wake callback is attached, and after one write there is one pending chunk and one wake.
#[test]
fn write_returns_before_embedding_and_wakes_drainer() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("async_write")?;
    let (e, _) = common::embedder("m1", 64);
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    let wake: embed_worker::Wake = Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&e)), Some(wake));
    notes.write(note("a", "apple pie"), false)?;
    assert_eq!(store.pending_count()?, 1, "write commits chunks without embedding them");
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    Ok(())
}

/// Proves the spawned background drainer embeds a write with no explicit drain. The note is written through a service wired to the drainer's wake. The test polls pending to zero with a 10 s deadline, then asserts search is no longer keyword-only.
#[test]
fn background_drainer_embeds_writes() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("drainer")?;
    let (e, _) = common::embedder("m1", 64);
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let drainer = embed_worker::spawn_drainer(Arc::clone(&store), Arc::clone(&e));
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&e)), Some(drainer.wake()));
    let searcher = SearchService::new(Arc::clone(&store), Some(Arc::clone(&e)));
    notes.write(note("a", "apple pie"), false)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while store.pending_count()? > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(store.pending_count()?, 0);
    assert!(!search(&searcher, "apple")?.keyword_only);
    Ok(())
}

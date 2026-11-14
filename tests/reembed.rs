//! Embedder change: vectors survive a mismatch, and re-embedding waits for confirmation.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use memhub_mcp::app::embed_worker;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::app::reembed::ReembedService;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::{NewNote, SearchFilter, SearchRequest, SearchResults};
use memhub_mcp::error::Result;
use memhub_mcp::ports::embedder::SharedEmbedder;
use memhub_mcp::ports::store::NoteStore;

/// Creates a store with notes `a` (`apple pie`) and `b` (`banana bread`), all embedded under the `model=old x=1` identity.
///
/// Side effects: creates the store through `common::store`, and drains the embed queue synchronously.
///
/// # Arguments
/// * `name` - store name passed to `common::store`.
///
/// # Returns
/// The store behind an `Arc`.
///
/// # Errors
/// * Propagates store, adoption, write, or drain errors.
fn embedded(name: &str) -> Result<Arc<dyn NoteStore>> {
    let store: Arc<dyn NoteStore> = common::store(name)?;
    let (e, _) = common::embedder("model=old x=1", 64);
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&e)), None);
    for (title, body) in [("a", "apple pie"), ("b", "banana bread")] {
        notes.write(NewNote { title: title.into(), body: body.into(), tags: vec![] }, false)?;
    }
    embed_worker::drain(store.as_ref(), &e, &mut |_| {})?;
    Ok(store)
}

/// Runs the query `apple` through `SearchService` using the given embedder.
///
/// # Arguments
/// * `store` - store to search.
/// * `e` - embedder used for the query.
///
/// # Returns
/// The search results.
///
/// # Errors
/// * Propagates search errors.
fn search(store: &Arc<dyn NoteStore>, e: &SharedEmbedder) -> Result<SearchResults> {
    let svc = SearchService::new(Arc::clone(store), Some(Arc::clone(e)));
    svc.search(&SearchRequest { query: "apple".into(), limit: 10, filter: SearchFilter::default() })
}

/// Counts stored embedding rows across all chunks.
///
/// # Arguments
/// * `store` - store to count.
///
/// # Returns
/// The number of stored embeddings.
///
/// # Errors
/// * Propagates iteration errors.
fn vectors(store: &dyn NoteStore) -> Result<usize> {
    let mut n = 0;
    store.for_each_embedding(&SearchFilter::default(), &mut |_, _, _| n += 1)?;
    Ok(n)
}

/// Collects every stored (chunk, vector) pair, sorted by chunk label, for byte-exact before/after comparison.
///
/// # Arguments
/// * `store` - store to snapshot.
///
/// # Returns
/// The sorted pairs.
///
/// # Errors
/// * Propagates iteration errors.
fn snapshot(store: &dyn NoteStore) -> Result<Vec<(String, Vec<f32>)>> {
    let mut all = Vec::new();
    store.for_each_embedding(&SearchFilter::default(), &mut |c, _, v| all.push((format!("{c:?}"), v.to_vec())))?;
    all.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(all)
}

/// Checks that a model mismatch keeps stored vectors byte-identical, degrades search to keyword-only with an `embedder_changed` notice, and that switching back to the old identity restores vector search.
///
/// Setup: two notes embedded by `old`, then a drainer on `new` is woken and given 200 ms.
///
/// # Errors
/// * Propagates store, search, and drain errors.
///
/// # Panics
/// Panics if the notice fields or the vector snapshot differ from expectations.
#[test]
fn mismatch_keeps_vectors_and_switching_back_restores_vector_search() -> Result<()> {
    let store = embedded("reembed_mismatch")?;
    let before = snapshot(store.as_ref())?;
    let (new, _) = common::embedder("model=new x=1", 64);
    let drainer = embed_worker::spawn_drainer(Arc::clone(&store), Arc::clone(&new));
    let wake = drainer.wake();
    wake();
    let r = search(&store, &new)?;
    assert!(r.keyword_only);
    let Some(c) = r.embedder_changed else { panic!("notice while mismatched") };
    assert_eq!((c.stored.as_str(), c.configured.as_str(), c.chunks), ("old", "new", 2));
    assert!(c.est_seconds > 0.0);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(snapshot(store.as_ref())?, before, "stored vectors byte-identical");
    assert_eq!(store.model_identity()?.as_deref(), Some("model=old x=1"));
    let (old, _) = common::embedder("model=old x=1", 64);
    let r = search(&store, &old)?;
    assert!(!r.keyword_only);
    assert!(r.embedder_changed.is_none());
    Ok(())
}

/// Checks that `reembed(false)` without confirmation starts nothing: it reports the mismatch and leaves the two vectors and the `old` identity untouched.
///
/// # Errors
/// * Propagates store and service errors.
///
/// # Panics
/// Panics on any mismatch.
#[test]
fn reembed_without_confirm_changes_nothing() -> Result<()> {
    let store = embedded("reembed_noconfirm")?;
    let (new, _) = common::embedder("model=new x=1", 64);
    let svc = ReembedService::new(Arc::clone(&store), Some(new), None);
    let s = svc.reembed(false)?;
    assert!(!s.started);
    assert!(s.embedder_changed.is_some());
    assert_eq!(vectors(store.as_ref())?, 2);
    assert_eq!(store.model_identity()?.as_deref(), Some("model=old x=1"));
    Ok(())
}

/// Checks that a confirmed `reembed(true)` re-embeds every note under `new`: the identity flips, the pending queue drains within 10 s, search becomes vector-based, and a second confirmation is a no-op.
///
/// # Errors
/// * Propagates store, service, and search errors.
///
/// # Panics
/// Panics on any mismatch.
#[test]
fn confirmed_reembed_embeds_everything_under_new_identity() -> Result<()> {
    let store = embedded("reembed_confirm")?;
    let (new, _) = common::embedder("model=new x=1", 64);
    let drainer = embed_worker::spawn_drainer(Arc::clone(&store), Arc::clone(&new));
    let wake = drainer.wake();
    let svc = ReembedService::new(Arc::clone(&store), Some(Arc::clone(&new)), Some(wake));
    let s = svc.reembed(true)?;
    assert!(s.started);
    assert_eq!(store.model_identity()?.as_deref(), Some("model=new x=1"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while store.pending_count()? > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(store.pending_count()?, 0);
    assert_eq!(vectors(store.as_ref())?, 2);
    let r = search(&store, &new)?;
    assert!(!r.keyword_only && r.embedder_changed.is_none());
    assert!(!svc.reembed(true)?.started, "second confirmation is a no-op");
    Ok(())
}

/// Checks that a note written while the store is on the old identity stays pending and is not embedded under the wrong identity, and that a confirmed re-embed then embeds it, giving 3 vectors.
///
/// # Errors
/// * Propagates store, service, and write errors.
///
/// # Panics
/// Panics on any mismatch, or if the queue does not drain within 10 s.
#[test]
fn note_written_while_mismatched_stays_pending_until_confirmed() -> Result<()> {
    let store = embedded("reembed_pending")?;
    let before = snapshot(store.as_ref())?;
    let (new, _) = common::embedder("model=new x=1", 64);
    let drainer = embed_worker::spawn_drainer(Arc::clone(&store), Arc::clone(&new));
    let wake = drainer.wake();
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&new)), Some(Arc::clone(&wake)));
    notes.write(NewNote { title: "c".into(), body: "cherry tart".into(), tags: vec![] }, false)?;
    wake();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(store.pending_count()?, 1, "no embedding under the wrong identity");
    assert_eq!(snapshot(store.as_ref())?, before, "stored vectors byte-identical");
    assert_eq!(store.model_identity()?.as_deref(), Some("model=old x=1"));
    assert!(ReembedService::new(Arc::clone(&store), Some(new), Some(wake)).reembed(true)?.started);
    let deadline = Instant::now() + Duration::from_secs(10);
    while store.pending_count()? > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(vectors(store.as_ref())?, 3);
    Ok(())
}

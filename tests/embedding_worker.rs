//! `nearest` scan and background `spawn_drainer` behaviour against a real store.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use memhub_mcp::app::embed_worker;
use memhub_mcp::app::nearest::nearest;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::domain::{NewNote, Note, SearchFilter, Tag};
use memhub_mcp::error::Result;
use memhub_mcp::ports::embedder::{Kind, SharedEmbedder};
use memhub_mcp::ports::store::NoteStore;

/// Creates a temp store and a fake embedder, adopts the embedder's model if the store is fresh, and
/// builds a note service that chunks with that embedder's tokenizer.
///
/// # Arguments
/// * `name` - Unique test name for the temp database.
/// * `identity` - Fake model identity recorded by the store.
/// * `max_tokens` - Fake tokenizer chunk size; small values split notes into several chunks.
///
/// # Returns
/// `(store, embedder, note service)`.
///
/// # Errors
/// Any store open or model-adoption error.
fn setup(name: &str, identity: &str, max_tokens: usize) -> Result<(Arc<dyn NoteStore>, SharedEmbedder, NoteService)> {
    let store: Arc<dyn NoteStore> = common::store(name)?;
    let (e, _) = common::embedder(identity, max_tokens);
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&e)), None);
    Ok((store, e, notes))
}

/// Writes a note with an optional single tag (no overwrite); its chunks start pending.
///
/// # Arguments
/// * `notes` - Note service under test.
/// * `title` - Note title.
/// * `body` - Note body.
/// * `tag` - Optional tag name; `None` writes an untagged note.
///
/// # Returns
/// The created note.
///
/// # Errors
/// Any error from [`NoteService::write`].
fn write(notes: &NoteService, title: &str, body: &str, tag: Option<&str>) -> Result<Note> {
    let tags = tag.map(|t| vec![Tag(t.into())]).unwrap_or_default();
    notes.write(NewNote { title: title.into(), body: body.into(), tags }, false)
}

/// Embeds `text` as a search query with the shared embedder, holding its lock for the call.
///
/// # Arguments
/// * `e` - Shared embedder.
/// * `text` - Query text.
///
/// # Returns
/// The query vector.
///
/// # Errors
/// Any embedder error.
fn query(e: &SharedEmbedder, text: &str) -> Result<Vec<f32>> {
    embed_worker::lock(e).embed(text, Kind::Query)
}

/// Polls every 10 ms until no chunk is pending or 10 seconds pass.
///
/// # Arguments
/// * `store` - Store whose pending-chunk count is polled.
///
/// # Returns
/// The pending-chunk count when polling stopped (0 means fully drained).
///
/// # Errors
/// Any store error while counting.
fn wait_drained(store: &dyn NoteStore) -> Result<usize> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while store.pending_count()? > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    store.pending_count()
}

/// Proves `nearest` returns one hit per note holding its best chunk, and honors `k`: a note split
/// into 3-token chunks ranks first with its "stripes" chunk, three notes give three hits, and `k = 2`
/// returns the two zebra notes in order.
#[test]
fn nearest_keeps_best_chunk_per_note_and_limits_to_k() -> Result<()> {
    let (store, e, notes) = setup("nearest_best", "m1", 3)?;
    let long = write(&notes, "long", "lorem ipsum dolor sit amet zebra stripes gallop", None)?;
    let other = write(&notes, "other", "zebra crossing", None)?;
    write(&notes, "far", "cats sleep", None)?;
    embed_worker::drain(store.as_ref(), &e, &mut |_| {})?;
    let hits = nearest(store.as_ref(), &SearchFilter::default(), &query(&e, "zebra stripes gallop")?, 10)?;
    assert_eq!(hits.len(), 3, "one hit per note, not per chunk: {hits:?}");
    assert_eq!(hits[0].0, long.id);
    let best = store.chunk_text(hits[0].2)?.unwrap_or_default();
    assert!(best.contains("stripes"), "best chunk kept, got {best:?}");
    let top2 = nearest(store.as_ref(), &SearchFilter::default(), &query(&e, "zebra stripes gallop")?, 2)?;
    assert_eq!(top2.iter().map(|h| h.0).collect::<Vec<_>>(), vec![long.id, other.id]);
    Ok(())
}

/// Proves `nearest` applies the tag filter and skips deleted notes and unembedded chunks: of four
/// "apple" notes only the embedded, live note tagged "food" is returned.
#[test]
fn nearest_filters_tag_and_skips_pending_and_deleted() -> Result<()> {
    let (store, e, notes) = setup("nearest_filter", "m1", 64)?;
    let tagged = write(&notes, "a", "apple pie", Some("food"))?;
    write(&notes, "b", "apple tart", None)?;
    let deleted = write(&notes, "c", "apple crumble", Some("food"))?;
    embed_worker::drain(store.as_ref(), &e, &mut |_| {})?;
    notes.delete(deleted.id)?;
    write(&notes, "d", "apple pie", Some("food"))?;
    assert_eq!(store.pending_count()?, 1, "note d stays unembedded");
    let filter = SearchFilter { tag: Some(Tag("food".into())), ..Default::default() };
    let hits = nearest(store.as_ref(), &filter, &query(&e, "apple pie")?, 10)?;
    assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), vec![tagged.id]);
    Ok(())
}

/// Proves the background drainer embeds chunks written after it started once woken: two new notes
/// leave 2 pending chunks and calling the wake handle drains them to 0.
#[test]
fn drainer_embeds_pending_chunks_after_wake() -> Result<()> {
    let (store, e, notes) = setup("drainer_wake", "m1", 64)?;
    let drainer = embed_worker::spawn_drainer(Arc::clone(&store), Arc::clone(&e));
    let wake = drainer.wake();
    assert_eq!(wait_drained(store.as_ref())?, 0);
    write(&notes, "a", "apple pie", None)?;
    write(&notes, "b", "banana bread", None)?;
    assert_eq!(store.pending_count()?, 2);
    wake();
    assert_eq!(wait_drained(store.as_ref())?, 0);
    Ok(())
}

/// Proves the drainer embeds nothing when its embedder's identity ("new") differs from the store's
/// model ("old"): after a wake and 300 ms the chunk is still pending.
#[test]
fn drainer_embeds_nothing_on_model_mismatch() -> Result<()> {
    let (store, _, notes) = setup("drainer_mismatch", "old", 64)?;
    write(&notes, "a", "apple pie", None)?;
    let (new, _) = common::embedder("new", 64);
    let drainer = embed_worker::spawn_drainer(Arc::clone(&store), new);
    let wake = drainer.wake();
    wake();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(store.pending_count()?, 1);
    Ok(())
}

/// Proves the drainer thread survives embedder failures: with the fake marked down the chunk stays
/// pending, and after the fake is restored a wake drains it.
#[test]
fn drainer_survives_embedder_failure() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("drainer_fail")?;
    let (e, down) = common::embedder("m1", 64);
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&e)), None);
    write(&notes, "a", "apple pie", None)?;
    down.store(true, Ordering::SeqCst);
    let drainer = embed_worker::spawn_drainer(Arc::clone(&store), Arc::clone(&e));
    let wake = drainer.wake();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(store.pending_count()?, 1, "failed embedding stays pending");
    down.store(false, Ordering::SeqCst);
    wake();
    assert_eq!(wait_drained(store.as_ref())?, 0, "thread still serves the next wake");
    Ok(())
}

//! Embedding backlog recovery: notes written without an embedder, embedder outages, and
//! interrupted requeues.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use memhub_mcp::app::embed_worker;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::app::reembed::ReembedService;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::{NewNote, Note, SearchFilter, SearchRequest};
use memhub_mcp::error::{Error, Result};
use memhub_mcp::ports::embedder::{Embedder, Kind, SharedEmbedder, Tokenizer};
use memhub_mcp::ports::store::NoteStore;

/// Writes one note through `notes`.
///
/// # Arguments
/// * `notes` - service to write with.
/// * `title` - note title.
/// * `body` - note body.
///
/// # Returns
/// The stored note.
///
/// # Errors
/// * Propagates write errors.
fn put(notes: &NoteService, title: &str, body: &str) -> Result<Note> {
    notes.write(NewNote { title: title.into(), body: body.into(), tags: vec![] }, false)
}

/// Lists the debug form of the note id of every stored embedding.
///
/// # Arguments
/// * `store` - store to scan.
///
/// # Returns
/// One entry per stored vector.
///
/// # Errors
/// * Propagates iteration errors.
fn embedded_notes(store: &dyn NoteStore) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    store.for_each_embedding(&SearchFilter::default(), &mut |_, n, _| ids.push(format!("{n:?}")))?;
    Ok(ids)
}

/// Waits up to 10 s for `store` to hold `n` embeddings.
///
/// # Arguments
/// * `store` - store to poll.
/// * `n` - expected vector count.
///
/// # Errors
/// * Propagates iteration errors.
fn wait_vectors(store: &dyn NoteStore, n: usize) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while embedded_notes(store)?.len() < n && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// Asserts that `note` is embedded and that a hybrid search for `apple` uses vectors and finds it.
///
/// # Arguments
/// * `store` - store to inspect.
/// * `e` - embedder for the query.
/// * `note` - the note expected to be embedded.
///
/// # Errors
/// * Propagates store and search errors.
fn assert_found(store: &Arc<dyn NoteStore>, e: &SharedEmbedder, note: &Note) -> Result<()> {
    assert_eq!(embedded_notes(store.as_ref())?, vec![format!("{:?}", note.id)]);
    let svc = SearchService::new(Arc::clone(store), Some(Arc::clone(e)));
    let r = svc.search(&SearchRequest { query: "apple".into(), limit: 10, filter: SearchFilter::default() })?;
    assert!(!r.keyword_only, "vector search active");
    assert!(r.hits.iter().any(|h| h.note_id == note.id));
    Ok(())
}

/// A note written with no embedder is chunked and embedded once the daemon starts with one.
///
/// # Errors
/// * Propagates store, write, and search errors.
#[test]
fn chunkless_note_is_embedded_at_startup() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("recovery_startup")?;
    let note = put(&NoteService::new(Arc::clone(&store), None, None), "a", "apple pie")?;
    let (e, _) = common::embedder("model=m x=1", 64);
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let _drainer = embed_worker::spawn_drainer(Arc::clone(&store), Arc::clone(&e));
    wait_vectors(store.as_ref(), 1)?;
    assert_found(&store, &e, &note)
}

/// A confirmed MCP reembed with an unchanged identity chunks notes written without an embedder,
/// reports them pending, and the drainer then embeds them.
///
/// # Errors
/// * Propagates store, service, and search errors.
#[test]
fn confirmed_reembed_chunks_chunkless_notes() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("recovery_reembed")?;
    let (e, _) = common::embedder("model=m x=1", 64);
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let note = put(&NoteService::new(Arc::clone(&store), None, None), "a", "apple pie")?;
    let s = ReembedService::new(Arc::clone(&store), Some(Arc::clone(&e)), None).reembed(true)?;
    assert!(s.started && s.embedder_changed.is_none());
    assert_eq!(s.pending_embeddings, 1);
    embed_worker::drain(store.as_ref(), &e, &mut |_| {})?;
    assert_found(&store, &e, &note)
}

/// Repeated drains during an embedder outage charge no attempts, so the backlog is embedded once
/// the embedder is back.
///
/// # Errors
/// * Propagates store and write errors.
#[test]
fn outage_does_not_exhaust_backlog() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("recovery_outage")?;
    let (e, down) = common::embedder("model=m x=1", 64);
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&e)), None);
    for i in 0..5 {
        put(&notes, &format!("n{i}"), &format!("note number {i}"))?;
    }
    down.store(true, Ordering::SeqCst);
    for _ in 0..4 {
        embed_worker::drain(store.as_ref(), &e, &mut |_| {})?;
    }
    assert_eq!((store.failed_count()?, store.pending_count()?), (0, 5));
    down.store(false, Ordering::SeqCst);
    assert_eq!(embed_worker::drain(store.as_ref(), &e, &mut |_| {})?, 5);
    Ok(())
}

/// A confirmed reembed with an unchanged identity clears given-up chunks so they are retried.
///
/// # Errors
/// * Propagates store and service errors.
#[test]
fn confirmed_reembed_retries_failed_chunks() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("recovery_failed")?;
    let (e, _) = common::embedder("model=m x=1", 64);
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    put(&NoteService::new(Arc::clone(&store), Some(Arc::clone(&e)), None), "a", "apple pie")?;
    let chunk = store.pending_chunks(memhub_mcp::domain::ChunkId(0), 1)?.remove(0).id;
    while !store.record_embed_failure(chunk)? {}
    assert_eq!((store.failed_count()?, store.pending_count()?), (1, 0));
    let s = ReembedService::new(Arc::clone(&store), Some(Arc::clone(&e)), None).reembed(true)?;
    assert!(s.started);
    assert_eq!((s.failed_embeddings, s.pending_embeddings), (0, 1));
    assert_eq!(embed_worker::drain(store.as_ref(), &e, &mut |_| {})?, 1);
    Ok(())
}

/// Embedder delegating to an inner one but failing on any text containing `poison`.
struct Poison(Box<dyn Embedder>);

impl Embedder for Poison {
    /// Delegates unless `text` contains `poison`.
    ///
    /// # Errors
    /// * [`Error::Embedder`] - `text` contains `poison`; otherwise the inner embedder's errors.
    fn embed(&mut self, text: &str, kind: Kind) -> Result<Vec<f32>> {
        if text.contains("poison") {
            return Err(Error::Embedder("cannot embed".into()));
        }
        self.0.embed(text, kind)
    }

    /// Delegates.
    fn dim(&self) -> usize {
        self.0.dim()
    }

    /// Delegates.
    fn identity(&self) -> String {
        self.0.identity()
    }

    /// Delegates.
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        self.0.tokenizer()
    }
}

/// Three always-failing chunks at the head of the queue are charged until they give up, and the
/// healthy chunks behind them are embedded; they are not mistaken for an outage.
///
/// # Errors
/// * Propagates store and write errors.
#[test]
fn poison_chunks_at_queue_head_do_not_stall_embedding() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("recovery_poison")?;
    let (e, _) = common::embedder("model=m x=1", 64);
    let inner = std::mem::replace(&mut *embed_worker::lock(&e), Box::new(Unreachable));
    let e: SharedEmbedder = Arc::new(std::sync::Mutex::new(Box::new(Poison(inner))));
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&e)), None);
    for i in 0..3 {
        put(&notes, &format!("p{i}"), &format!("poison {i}"))?;
    }
    put(&notes, "g0", "good apple")?;
    put(&notes, "g1", "good pear")?;
    let mut drained = Vec::new();
    for _ in 0..4 {
        drained.push(embed_worker::drain(store.as_ref(), &e, &mut |_| {})?);
    }
    assert_eq!(drained, vec![2, 0, 0, 0]);
    assert_eq!((store.failed_count()?, store.pending_count()?), (3, 0));
    Ok(())
}

/// Notes whose body is only whitespace or invisible characters get no chunks, so they are not
/// picked up again on every drain and a confirmed reembed does not report them as started.
///
/// # Errors
/// * Propagates store, write, and service errors.
#[test]
fn blank_bodies_are_not_rechunked_forever() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("recovery_blank")?;
    let (e, _) = common::embedder("model=m x=1", 64);
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let notes = NoteService::new(Arc::clone(&store), None, None);
    let bodies = ["\n\n", "\t", "\r\n", "\u{a0}", "\u{200b}", "\u{3000}", "\u{feff}", "\0", " \n\t\u{a0}\u{200b} "];
    for (i, body) in bodies.iter().enumerate() {
        put(&notes, &format!("b{i}"), body)?;
    }
    assert_eq!(store.chunkless_notes()?, vec![], "blank bodies are never chunkless work");
    assert_eq!(embed_worker::chunk_missing(store.as_ref(), &e)?, 0);
    assert!(!ReembedService::new(Arc::clone(&store), Some(Arc::clone(&e)), None).reembed(true)?.started);
    Ok(())
}

/// Tokenizer delegating to an inner one but panicking once, on the first call.
struct PanicOnce(Arc<dyn Tokenizer>, Arc<std::sync::atomic::AtomicBool>);

impl Tokenizer for PanicOnce {
    /// Panics on the first call, then delegates.
    ///
    /// # Errors
    /// * Propagates the inner tokenizer's errors.
    fn token_spans(&self, text: &str) -> Result<Vec<(usize, usize)>> {
        assert!(!self.1.swap(false, Ordering::SeqCst), "tokenizer panicked");
        self.0.token_spans(text)
    }

    /// Delegates.
    fn max_tokens(&self) -> usize {
        self.0.max_tokens()
    }
}

/// Embedder delegating to an inner one but handing out a [`PanicOnce`] tokenizer.
struct PanicOnceEmbedder(Box<dyn Embedder>, Arc<std::sync::atomic::AtomicBool>);

impl Embedder for PanicOnceEmbedder {
    /// Delegates.
    ///
    /// # Errors
    /// * Propagates the inner embedder's errors.
    fn embed(&mut self, text: &str, kind: Kind) -> Result<Vec<f32>> {
        self.0.embed(text, kind)
    }

    /// Delegates.
    fn dim(&self) -> usize {
        self.0.dim()
    }

    /// Delegates.
    fn identity(&self) -> String {
        self.0.identity()
    }

    /// Wraps the inner tokenizer in [`PanicOnce`].
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        Arc::new(PanicOnce(self.0.tokenizer(), Arc::clone(&self.1)))
    }
}

/// A panic while the background drainer chunks notes does not kill its thread: a later wake
/// still embeds the backlog.
///
/// # Errors
/// * Propagates store and write errors.
#[test]
fn drainer_survives_a_panic() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("recovery_panic")?;
    let (e, _) = common::embedder("model=m x=1", 64);
    let inner = std::mem::replace(&mut *embed_worker::lock(&e), Box::new(Unreachable));
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let e: SharedEmbedder = Arc::new(std::sync::Mutex::new(Box::new(PanicOnceEmbedder(inner, Arc::clone(&armed)))));
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    put(&NoteService::new(Arc::clone(&store), None, None), "a", "apple pie")?;
    let drainer = embed_worker::spawn_drainer(Arc::clone(&store), Arc::clone(&e));
    let deadline = Instant::now() + Duration::from_secs(10);
    while armed.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
    (drainer.wake())();
    wait_vectors(store.as_ref(), 1)?;
    assert_eq!(embedded_notes(store.as_ref())?.len(), 1, "drainer thread still alive");
    Ok(())
}

/// Tokenizer that fails on any text containing `banana`.
struct Picky(Arc<dyn Tokenizer>);

impl Tokenizer for Picky {
    /// Delegates, failing on `banana`.
    ///
    /// # Errors
    /// * [`Error::Embedder`] - `text` contains `banana`.
    fn token_spans(&self, text: &str) -> Result<Vec<(usize, usize)>> {
        if text.contains("banana") {
            return Err(Error::Embedder("tokenizer crashed".into()));
        }
        self.0.token_spans(text)
    }

    /// Delegates.
    fn max_tokens(&self) -> usize {
        self.0.max_tokens()
    }
}

/// Embedder delegating to an inner one but handing out a [`Picky`] tokenizer.
struct PickyEmbedder(Box<dyn Embedder>);

impl Embedder for PickyEmbedder {
    /// Delegates.
    ///
    /// # Errors
    /// * Propagates the inner embedder's errors.
    fn embed(&mut self, text: &str, kind: Kind) -> Result<Vec<f32>> {
        self.0.embed(text, kind)
    }

    /// Delegates.
    fn dim(&self) -> usize {
        self.0.dim()
    }

    /// Delegates.
    fn identity(&self) -> String {
        self.0.identity()
    }

    /// Wraps the inner tokenizer in [`Picky`].
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        Arc::new(Picky(self.0.tokenizer()))
    }
}

/// A requeue interrupted part-way keeps the old identity, so the change is still detected and
/// the requeue retried.
///
/// # Errors
/// * Propagates store and write errors.
#[test]
fn interrupted_requeue_keeps_old_identity() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("recovery_requeue")?;
    let (old, _) = common::embedder("model=old x=1", 64);
    embed_worker::adopt_if_fresh(store.as_ref(), &old)?;
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&old)), None);
    put(&notes, "a", "apple pie")?;
    put(&notes, "b", "banana bread")?;
    let (new, _) = common::embedder("model=new x=1", 64);
    let inner = std::mem::replace(&mut *embed_worker::lock(&new), Box::new(Unreachable));
    let picky: SharedEmbedder = Arc::new(std::sync::Mutex::new(Box::new(PickyEmbedder(inner))));
    assert!(embed_worker::requeue(store.as_ref(), &picky).is_err());
    assert_eq!(store.model_identity()?.as_deref(), Some("model=old x=1"));
    Ok(())
}

/// Placeholder left in the emptied shared embedder; never called.
struct Unreachable;

impl Embedder for Unreachable {
    /// Never called.
    fn embed(&mut self, _: &str, _: Kind) -> Result<Vec<f32>> {
        unreachable!()
    }

    /// Never called.
    fn dim(&self) -> usize {
        unreachable!()
    }

    /// Never called.
    fn identity(&self) -> String {
        unreachable!()
    }

    /// Never called.
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        unreachable!()
    }
}

//! The embedder failing, panicking or being slow never blocks search, writes or other chunks.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{FakeEmbedder, FakeTokenizer};
use memhub_mcp::app::embed_worker;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::app::reembed::ReembedService;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::{NewNote, SearchFilter, SearchRequest, SearchResults};
use memhub_mcp::error::{Error, Result};
use memhub_mcp::ports::embedder::{Embedder, Kind, SharedEmbedder, Tokenizer};
use memhub_mcp::ports::store::NoteStore;

/// A [`FakeEmbedder`] that can panic once, always fails on texts containing "poison", sleeps
/// `delay` per call and logs every call.
struct Scripted {
    inner: FakeEmbedder,
    panic_once: Arc<AtomicBool>,
    delay: Duration,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Embedder for Scripted {
    /// Embeds `text` with scripted misbehavior: panics once if `panic_once` is armed (clearing it),
    /// fails for texts containing "poison", otherwise logs "start", sleeps `delay`, logs "end" and
    /// delegates to the inner [`FakeEmbedder`].
    ///
    /// # Arguments
    /// * `text` - Text to embed; "poison" anywhere in it forces a failure.
    /// * `kind` - Query or document embedding, passed through to the inner fake.
    ///
    /// # Returns
    /// The inner fake's vector.
    ///
    /// # Errors
    /// * [`Error::Embedder`] - `text` contains "poison", or the inner fake fails (e.g. it is marked down).
    ///
    /// # Panics
    /// Panics with "onnx exploded" on the first call after `panic_once` is set.
    fn embed(&mut self, text: &str, kind: Kind) -> Result<Vec<f32>> {
        if self.panic_once.swap(false, Ordering::SeqCst) {
            panic!("onnx exploded");
        }
        if text.contains("poison") {
            return Err(Error::Embedder("cannot embed".into()));
        }
        push(&self.log, "start");
        std::thread::sleep(self.delay);
        push(&self.log, "end");
        self.inner.embed(text, kind)
    }
    /// Returns the inner fake's vector dimension.
    ///
    /// # Returns
    /// Number of floats per embedding.
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    /// Returns the inner fake's model identity (e.g. "m1"), used by the store to detect model changes.
    ///
    /// # Returns
    /// The model identity string.
    fn identity(&self) -> String {
        self.inner.identity()
    }
    /// Returns the inner fake's tokenizer, used for chunking note text.
    ///
    /// # Returns
    /// Shared tokenizer handle.
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        self.inner.tokenizer()
    }
}

/// Appends `event` to the call log, silently skipping it if the mutex is poisoned.
///
/// # Arguments
/// * `log` - Shared call log written by [`Scripted::embed`] and read by the tests.
/// * `event` - Event label, "start" or "end".
fn push(log: &Mutex<Vec<&'static str>>, event: &'static str) {
    if let Ok(mut l) = log.lock() {
        l.push(event);
    }
}

/// Everything a resilience test needs: the store, the shared scripted embedder and its controls, and services.
struct Setup {
    store: Arc<dyn NoteStore>,
    embedder: SharedEmbedder,
    panic_once: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<&'static str>>>,
    notes: NoteService,
    search: SearchService,
}

/// Builds a fresh temp store, a [`Scripted`] embedder with identity "m1" (64-token fake tokenizer),
/// adopts it as the store's model if the store is fresh, and creates note and search services on top.
///
/// # Arguments
/// * `name` - Unique test name used for the temp database path.
/// * `delay` - Sleep per successful embed call, to simulate a slow model.
///
/// # Returns
/// The assembled [`Setup`]; `panic_once` starts disarmed and the log empty.
///
/// # Errors
/// Any store open or model-adoption error.
fn setup(name: &str, delay: Duration) -> Result<Setup> {
    let store: Arc<dyn NoteStore> = common::store(name)?;
    let panic_once = Arc::new(AtomicBool::new(false));
    let log = Arc::new(Mutex::new(Vec::new()));
    let inner = FakeEmbedder {
        identity: "m1".into(),
        tokenizer: Arc::new(FakeTokenizer { max_tokens: 64 }),
        down: Arc::new(AtomicBool::new(false)),
    };
    let scripted = Scripted { inner, panic_once: Arc::clone(&panic_once), delay, log: Arc::clone(&log) };
    let embedder: SharedEmbedder = Arc::new(Mutex::new(Box::new(scripted)));
    embed_worker::adopt_if_fresh(store.as_ref(), &embedder)?;
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&embedder)), None);
    let search = SearchService::new(Arc::clone(&store), Some(Arc::clone(&embedder)));
    Ok(Setup { store, embedder, panic_once, log, notes, search })
}

/// Writes an untagged note with `title` and `body` through the note service (no overwrite).
///
/// # Arguments
/// * `notes` - Service that writes and chunks the note.
/// * `title` - Note title.
/// * `body` - Note body.
///
/// # Errors
/// Any error from [`NoteService::write`].
fn write(notes: &NoteService, title: &str, body: &str) -> Result<()> {
    notes.write(NewNote { title: title.into(), body: body.into(), tags: Vec::new() }, false).map(|_| ())
}

/// Runs an unfiltered search for `query` with a limit of 10 hits.
///
/// # Arguments
/// * `svc` - Search service under test.
/// * `query` - Search text.
///
/// # Returns
/// The search results, including keyword-only flag and pending/failed embedding counts.
///
/// # Errors
/// Any error from [`SearchService::search`].
fn search(svc: &SearchService, query: &str) -> Result<SearchResults> {
    svc.search(&SearchRequest { query: query.into(), limit: 10, filter: SearchFilter::default() })
}

/// Proves an embedder panic never breaks the server: a panic while holding the embedder lock poisons
/// the mutex, yet keyword search still answers, a panicking query embed falls back to keyword-only,
/// and the drain worker then recovers the lock, embeds the pending chunk and vector search works again.
#[test]
fn embedder_panic_leaves_search_and_embedding_working() -> Result<()> {
    let s = setup("resilience_panic", Duration::ZERO)?;
    write(&s.notes, "a", "apple pie")?;
    // A panic while holding the lock poisons the mutex.
    s.panic_once.store(true, Ordering::SeqCst);
    let e = Arc::clone(&s.embedder);
    let crashed = std::thread::spawn(move || embed_worker::lock(&e).embed("x", Kind::Query)).join();
    assert!(crashed.is_err() && s.embedder.is_poisoned());

    let r = search(&s.search, "apple")?;
    assert_eq!(r.hits.len(), 1, "keyword search still answers");
    s.panic_once.store(true, Ordering::SeqCst);
    let r = search(&s.search, "apple")?;
    assert!(r.keyword_only && r.hits.len() == 1, "a panicking query embed falls back to keyword");

    assert_eq!(embed_worker::drain(s.store.as_ref(), &s.embedder, &mut |_| {})?, 1);
    assert!(!search(&s.search, "apple")?.keyword_only, "the recovered embedder serves vector search");
    Ok(())
}

/// Proves a chunk the embedder rejects ("poison") does not block other chunks: one drain embeds the
/// "apple" chunk, the poison chunk stays pending while retries remain, and after five more drains it is
/// counted as failed while its note is still found by keyword search.
#[test]
fn failing_chunk_does_not_block_later_ones_and_is_reported() -> Result<()> {
    let s = setup("resilience_poison", Duration::ZERO)?;
    write(&s.notes, "p", "poison")?;
    write(&s.notes, "a", "apple pie")?;
    assert_eq!(embed_worker::drain(s.store.as_ref(), &s.embedder, &mut |_| {})?, 1, "apple embeds past poison");
    let r = search(&s.search, "apple")?;
    assert_eq!((r.pending_embeddings, r.failed_embeddings), (1, 0), "poison is retried before giving up");
    for _ in 0..5 {
        embed_worker::drain(s.store.as_ref(), &s.embedder, &mut |_| {})?;
    }
    let r = search(&s.search, "apple")?;
    assert_eq!((r.pending_embeddings, r.failed_embeddings), (0, 1), "a hopeless chunk is counted as failed");
    assert_eq!(search(&s.search, "poison")?.hits.len(), 1, "and its note is still found by keyword");
    Ok(())
}

/// Proves re-embedding with a new model ("m2") gives permanently failed chunks fresh attempts: after
/// six drains the poison chunk is failed, and `reembed(true)` reports it as pending (1) with 0 failed.
#[test]
fn reembed_resets_the_failed_attempt_counter() -> Result<()> {
    let s = setup("resilience_reset", Duration::ZERO)?;
    write(&s.notes, "p", "poison")?;
    for _ in 0..6 {
        embed_worker::drain(s.store.as_ref(), &s.embedder, &mut |_| {})?;
    }
    let r = search(&s.search, "poison")?;
    assert_eq!((r.pending_embeddings, r.failed_embeddings), (0, 1));
    let m2 = FakeEmbedder {
        identity: "m2".into(),
        tokenizer: Arc::new(FakeTokenizer { max_tokens: 64 }),
        down: Arc::new(AtomicBool::new(false)),
    };
    let m2: SharedEmbedder = Arc::new(Mutex::new(Box::new(m2)));
    let status = ReembedService::new(Arc::clone(&s.store), Some(m2), None).reembed(true)?;
    assert!(status.started);
    assert_eq!(
        (status.pending_embeddings, status.failed_embeddings),
        (1, 0),
        "reembed gives failed chunks new attempts"
    );
    Ok(())
}

/// Proves `reset_embeddings` on the store clears the failed-attempt counter in place: a failed poison
/// chunk becomes pending again (1 pending, 0 failed) without the note being rechunked.
#[test]
fn reset_embeddings_clears_failed_attempts_in_place() -> Result<()> {
    let s = setup("resilience_reset_store", Duration::ZERO)?;
    write(&s.notes, "p", "poison")?;
    for _ in 0..6 {
        embed_worker::drain(s.store.as_ref(), &s.embedder, &mut |_| {})?;
    }
    assert_eq!(s.store.failed_count()?, 1);
    s.store.reset_embeddings("m2")?;
    assert_eq!((s.store.pending_count()?, s.store.failed_count()?), (1, 0), "attempts reset without rechunking");
    Ok(())
}

/// Proves dropping the background drainer waits for the in-flight embed to finish and then stops:
/// with 20 notes and a 50 ms embed delay, the log ends with "end" when drop returns, stays unchanged
/// 200 ms later, and chunks remain pending (shutdown did not drain the whole backlog).
#[test]
fn dropping_the_drainer_joins_it_before_returning() -> Result<()> {
    let s = setup("resilience_shutdown", Duration::from_millis(50))?;
    for i in 0..20 {
        write(&s.notes, &format!("n{i}"), &format!("word{i}"))?;
    }
    let drainer = embed_worker::spawn_drainer(Arc::clone(&s.store), Arc::clone(&s.embedder));
    while s.log.lock().map_err(|e| Error::Embedder(e.to_string()))?.is_empty() {
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(drainer);
    let at_drop = s.log.lock().map_err(|e| Error::Embedder(e.to_string()))?.clone();
    assert_eq!(at_drop.last(), Some(&"end"), "the chunk in flight finished before drop returned");
    std::thread::sleep(Duration::from_millis(200));
    let later = s.log.lock().map_err(|e| Error::Embedder(e.to_string()))?.clone();
    assert_eq!(later, at_drop, "nothing touches the embedder after shutdown");
    assert!(s.store.pending_count()? > 0, "shutdown did not wait for the whole backlog");
    Ok(())
}

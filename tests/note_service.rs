//! `NoteService` edits, validation and reads against a real store.

mod common;

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use memhub_mcp::app::embed_worker;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::domain::{EditOp, MAX_BODY, NewNote, Note, NoteId, Tag};
use memhub_mcp::error::{Error, Result};
use memhub_mcp::ports::embedder::{Embedder, Kind, SharedEmbedder, Tokenizer};

/// Embedder whose `embed` reports entry, then blocks until released.
struct BlockedEmbedder {
    entered: Sender<()>,
    release: Receiver<()>,
    tokenizer: Arc<common::FakeTokenizer>,
}

impl Embedder for BlockedEmbedder {
    /// Signals entry on `entered`, then blocks until `release` receives a message. Holds an embedding in flight.
    ///
    /// # Arguments
    /// * `_` - input text, ignored.
    /// * `_` - embedding kind, ignored.
    ///
    /// # Returns
    /// `Ok(vec![1.0])` once released.
    ///
    /// # Errors
    /// * Never fails; channel errors are discarded.
    fn embed(&mut self, _: &str, _: Kind) -> Result<Vec<f32>> {
        let _ = self.entered.send(());
        let _ = self.release.recv();
        Ok(vec![1.0])
    }
    /// Returns the constant dimension 1.
    ///
    /// # Returns
    /// `1`.
    fn dim(&self) -> usize {
        1
    }
    /// Returns the identity string `blocked`.
    ///
    /// # Returns
    /// `"blocked"`.
    fn identity(&self) -> String {
        "blocked".into()
    }
    /// Returns a shared handle to the fake tokenizer.
    ///
    /// # Returns
    /// The tokenizer behind a trait object.
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        Arc::clone(&self.tokenizer) as Arc<dyn Tokenizer>
    }
}

/// Checks that a note write completes while the embedder is blocked mid-inference.
///
/// Setup: a blocked embedder is held inside a drainer thread, then a writer thread calls `NoteService::write`. Asserts the write finishes within 10 s with title `W`, before the embedder is released.
///
/// # Errors
/// * Propagates store and write errors.
///
/// # Panics
/// Panics if the write times out or the drainer thread panics.
#[test]
fn write_does_not_wait_for_inference() -> Result<()> {
    let ((entered_tx, entered), (release, release_rx)) = (mpsc::channel(), mpsc::channel());
    let tokenizer = Arc::new(common::FakeTokenizer { max_tokens: 8 });
    let fake = BlockedEmbedder { entered: entered_tx, release: release_rx, tokenizer };
    let e: SharedEmbedder = Arc::new(Mutex::new(Box::new(fake)));
    let svc = Arc::new(NoteService::new(common::store("write_not_blocked")?, Some(Arc::clone(&e)), None));
    let drainer = std::thread::spawn(move || embed_worker::lock(&e).embed("x", Kind::Document).map(|_| ()));
    entered.recv().map_err(|e| Error::Embedder(e.to_string()))?;
    let (done_tx, done) = mpsc::channel();
    let writer = Arc::clone(&svc);
    std::thread::spawn(move || {
        done_tx.send(writer.write(NewNote { title: "W".into(), body: "a b".into(), tags: vec![] }, false))
    });
    let written = done.recv_timeout(Duration::from_secs(10));
    let _ = release.send(());
    assert_eq!(written.map_err(|e| Error::Storage(e.to_string()))??.title, "W");
    drainer.join().map_err(|_| Error::Embedder("drainer panicked".into()))?
}

/// Creates a store named `name` and writes note `T` with body `# A\na\n# B\nb\n` through `NoteService`, without an embedder.
///
/// Side effects: creates the store through `common::store`.
///
/// # Arguments
/// * `name` - store name passed to `common::store`.
///
/// # Returns
/// The service and the created note.
///
/// # Errors
/// * Propagates store or write errors.
fn setup(name: &str) -> Result<(NoteService, Note)> {
    let svc = NoteService::new(common::store(name)?, None, None);
    let note = NewNote { title: "T".into(), body: "# A\na\n# B\nb\n".into(), tags: vec![] };
    let created = svc.write(note, false)?;
    Ok((svc, created))
}

/// Applies each edit op in turn (replace section, append, set tags, replace body) and checks the body, tags, and version after each step.
///
/// Asserts the exact body strings and that the final version is 5.
///
/// # Errors
/// * Propagates edit errors.
///
/// # Panics
/// Panics on any mismatch.
#[test]
fn every_edit_op() -> Result<()> {
    let (svc, n) = setup("every_edit_op")?;
    let n = svc.edit(n.id, 1, EditOp::ReplaceSection { heading: "A".into(), content: "x".into() })?;
    assert_eq!(n.body, "# A\nx\n# B\nb\n");
    let n = svc.edit(n.id, 2, EditOp::Append("tail".into()))?;
    assert_eq!(n.body, "# A\nx\n# B\nb\ntail");
    let n = svc.edit(n.id, 3, EditOp::SetTags(vec![Tag("k".into())]))?;
    assert_eq!((n.tags.len(), n.body.len()), (1, 16));
    let n = svc.edit(n.id, 4, EditOp::ReplaceBody("new".into()))?;
    assert_eq!((n.body.as_str(), n.version, n.tags.len()), ("new", 5, 1));
    Ok(())
}

/// Checks the edit failure modes: a missing heading returns `InvalidInput`, a stale `expected_version` returns `VersionMismatch` (expected 9, actual 1), and an unknown id returns `NotFound`.
///
/// # Errors
/// * Propagates setup errors.
///
/// # Panics
/// Panics if any error variant differs.
#[test]
fn edit_errors() -> Result<()> {
    let (svc, n) = setup("edit_errors")?;
    let missing = EditOp::ReplaceSection { heading: "Z".into(), content: "x".into() };
    assert!(matches!(svc.edit(n.id, 1, missing), Err(Error::InvalidInput(_))));
    let stale = svc.edit(n.id, 9, EditOp::Append("x".into()));
    assert!(matches!(stale, Err(Error::VersionMismatch { expected: 9, actual: 1 })));
    assert!(matches!(svc.edit(NoteId(99), 1, EditOp::Append("x".into())), Err(Error::NotFound)));
    Ok(())
}

/// Checks that a tags-only edit bumps the version but keeps `updated_at`, while a body edit advances it.
///
/// Sleeps 1.1 s so timestamps differ at second resolution. Asserts version 2 after the tag edit.
///
/// # Errors
/// * Propagates setup and edit errors.
///
/// # Panics
/// Panics on any mismatch.
#[test]
fn tags_only_edit_keeps_updated_at() -> Result<()> {
    let (svc, n) = setup("tags_only_edit_keeps_updated_at")?;
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let tagged = svc.edit(n.id, 1, EditOp::SetTags(vec![Tag("k".into())]))?;
    assert_eq!((tagged.updated_at, tagged.version, tagged.tags.len()), (n.updated_at, 2, 1));
    let edited = svc.edit(n.id, 2, EditOp::Append("x".into()))?;
    assert!(edited.updated_at > n.updated_at);
    Ok(())
}

/// Checks title validation: blank and 201-character titles are rejected, 200 multibyte characters are accepted, and reads and deletes work by title.
///
/// # Errors
/// * Propagates setup, read, and delete errors.
///
/// # Panics
/// Panics on any mismatch.
#[test]
fn validates_titles_and_reads() -> Result<()> {
    let (svc, n) = setup("validates_titles_and_reads")?;
    let bad = |title: String| svc.write(NewNote { title, body: String::new(), tags: vec![] }, false);
    assert!(matches!(bad("  ".into()), Err(Error::InvalidInput(_))));
    assert!(matches!(bad("é".repeat(201)), Err(Error::InvalidInput(_))));
    assert!(bad("é".repeat(200)).is_ok());
    assert_eq!(svc.read_title("t")?.id, n.id);
    svc.delete(n.id)?;
    assert!(matches!(svc.read_title("T"), Err(Error::NotFound)));
    Ok(())
}

/// Checks the `MAX_BODY` cap on write and on every edit op: oversized bodies return `TooLarge` and leave the note at version 1, while a body of exactly `MAX_BODY` bytes is accepted.
///
/// # Errors
/// * Propagates setup errors.
///
/// # Panics
/// Panics on any mismatch.
#[test]
fn rejects_bodies_over_the_size_cap() -> Result<()> {
    let (svc, n) = setup("rejects_bodies_over_the_size_cap")?;
    let too_big = |r: Result<Note>| matches!(r, Err(Error::TooLarge(len)) if len > MAX_BODY);
    let big = NewNote { title: "Big".into(), body: "x".repeat(MAX_BODY + 1), tags: vec![] };
    assert!(too_big(svc.write(big, false)));
    assert!(matches!(svc.read_title("Big"), Err(Error::NotFound)));
    assert!(too_big(svc.edit(n.id, 1, EditOp::ReplaceBody("x".repeat(MAX_BODY + 1)))));
    assert!(too_big(svc.edit(n.id, 1, EditOp::Append("x".repeat(MAX_BODY)))));
    let section = EditOp::ReplaceSection { heading: "A".into(), content: "x".repeat(MAX_BODY) };
    assert!(too_big(svc.edit(n.id, 1, section)));
    assert_eq!(svc.read_id(n.id)?.version, 1, "rejected edits leave the note untouched");
    let fits = NewNote { title: "Fits".into(), body: "x".repeat(MAX_BODY), tags: vec![] };
    assert!(svc.write(fits, false).is_ok());
    Ok(())
}

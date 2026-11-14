//! A rechunk computed from a note body read before a concurrent edit must not overwrite the
//! chunks of the edited body.

mod common;

use memhub_mcp::domain::{NewChunk, NewNote};
use memhub_mcp::error::{Error, Result};
use memhub_mcp::ports::store::{EmbeddingQueue, NoteRecords};

/// One chunk spanning all of `body`.
///
/// # Arguments
/// * `body` - chunk text.
///
/// # Returns
/// A single-chunk list.
fn whole(body: &str) -> Vec<NewChunk> {
    vec![NewChunk { seq: 0, start: 0, end: body.len(), text: body.into() }]
}

/// Requeue reads a note, an edit lands, then requeue rechunks with the stale body: the stored
/// chunks must still be those of the edited body.
#[test]
fn stale_rechunk_keeps_edited_chunks() -> Result<()> {
    let store = common::store("rechunk_race")?;
    let note = NewNote { title: "t".into(), body: "old body".into(), tags: vec![] };
    let created = store.create(&note, &whole("old body"), false)?;
    let read = store.read_by_id(created.id)?.ok_or(Error::NotFound)?;
    store.edit(created.id, created.version, "new body", &[], &whole("new body"))?;
    store.rechunk(read.id, &read.body, &whole(&read.body))?;
    let texts: Vec<String> =
        store.pending_chunks(memhub_mcp::domain::ChunkId(0), 10)?.into_iter().map(|c| c.text).collect();
    assert_eq!(texts, vec!["new body".to_string()]);
    Ok(())
}

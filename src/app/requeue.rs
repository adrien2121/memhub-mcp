//! Rebuilds every chunk and embedding after the user confirms an embedder change.

use crate::app::chunk;
use crate::app::embed_worker::{drain, lock};
use crate::domain::{Cursor, ListQuery};
use crate::error::Result;
use crate::ports::embedder::SharedEmbedder;
use crate::ports::store::NoteStore;

/// Rebuilds all vector data in the calling thread: [`requeue`] then [`drain`]. Used by the
/// `reembed` command-line subcommand, which blocks until done.
///
/// # Arguments
/// * `store` - Store to rebuild.
/// * `embedder` - The new embedder whose identity is recorded.
/// * `progress` - Called with (embeddings stored so far, chunks pending when embedding began).
///
/// # Returns
/// The number of embeddings stored.
///
/// # Errors
/// * [`Error::Embedder`](crate::error::Error::Embedder) - Tokenising a note fails.
/// * [`Error::Storage`](crate::error::Error::Storage) - Any store call fails.
pub fn reembed(
    store: &dyn NoteStore,
    embedder: &SharedEmbedder,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<usize> {
    let total = requeue(store, embedder)?;
    drain(store, embedder, &mut |done| progress(done, total))
}

/// Re-chunks every live note (100 per page) with `embedder`'s tokenizer, then drops all
/// embeddings and records `embedder`'s identity, leaving every chunk pending for the drainer.
/// The identity is recorded last, so a failure part-way leaves the old identity in place and
/// the change is still detected and retried. Locks the embedder only to read identity and
/// tokenizer.
///
/// # Arguments
/// * `store` - Store to reset and re-chunk.
/// * `embedder` - The new embedder.
///
/// # Returns
/// The number of chunks now pending.
///
/// # Errors
/// * [`Error::Embedder`](crate::error::Error::Embedder) - Tokenising a note fails.
/// * [`Error::Storage`](crate::error::Error::Storage) - Any store call fails.
pub fn requeue(store: &dyn NoteStore, embedder: &SharedEmbedder) -> Result<usize> {
    let (identity, tokenizer) = {
        let e = lock(embedder);
        (e.identity(), e.tokenizer())
    };
    let mut cursor: Option<Cursor> = None;
    loop {
        let page = store.list(&ListQuery { limit: 100, cursor, ..Default::default() })?;
        for note in &page.items {
            let chunks = chunk::split(&note.body, tokenizer.as_ref())?;
            store.rechunk(note.id, &note.body, &chunks)?;
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    store.reset_embeddings(&identity)?;
    store.pending_count()
}

/// Chunks every live note that has a body but no chunks (written while no embedder was
/// configured), leaving the new chunks pending.
///
/// # Arguments
/// * `store` - Store holding the notes.
/// * `embedder` - The configured embedder; locked briefly for its tokenizer only if a note needs chunking.
///
/// # Returns
/// The number of notes that got at least one chunk.
///
/// # Errors
/// * [`Error::Embedder`](crate::error::Error::Embedder) - Tokenising a note fails.
/// * [`Error::Storage`](crate::error::Error::Storage) - Any store call fails.
pub fn chunk_missing(store: &dyn NoteStore, embedder: &SharedEmbedder) -> Result<usize> {
    let ids = store.chunkless_notes()?;
    if ids.is_empty() {
        return Ok(0);
    }
    let tokenizer = lock(embedder).tokenizer();
    let mut chunked = 0;
    for &id in &ids {
        if let Some(note) = store.read_by_id(id)? {
            let chunks = chunk::split(&note.body, tokenizer.as_ref())?;
            store.rechunk(id, &note.body, &chunks)?;
            chunked += usize::from(!chunks.is_empty());
        }
    }
    Ok(chunked)
}

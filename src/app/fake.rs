//! In-memory `NoteStore` for app unit tests, split by store trait into submodules.

mod queue;
mod records;
mod search;

use std::sync::atomic::AtomicUsize;
use std::sync::{Mutex, MutexGuard};

use crate::domain::{Chunk, Note};
use crate::error::{Error, Result};

/// Notes kept in a vector; `chunks` is a pending-chunk queue tests fill directly, emptied as
/// chunks are embedded. `fts_search` matches any quoted term as a case-insensitive substring of
/// a body. Public fields let tests inspect calls and inject failures.
#[derive(Default)]
pub struct FakeStore {
    notes: Mutex<Vec<Note>>,
    /// Last query passed to `fts_search`.
    pub last_query: Mutex<Option<String>>,
    /// Number of `fts_snippet` calls.
    pub snippets: AtomicUsize,
    /// Recorded embedder identity.
    pub identity: Mutex<Option<String>>,
    /// Calls to `pending_chunks` so far.
    pub pending_calls: AtomicUsize,
    /// How many more `pending_chunks` calls fail with `Storage`.
    pub pending_failures: AtomicUsize,
    /// Pending chunks, in ascending id order.
    pub chunks: Mutex<Vec<Chunk>>,
    /// Total chunk rows returned by `pending_chunks` so far.
    pub fetched: AtomicUsize,
}

impl FakeStore {
    /// Locks the note list, blocking until it is free.
    ///
    /// # Returns
    /// The guard over all stored notes.
    ///
    /// # Errors
    /// * [`Error::Storage`] - The mutex is poisoned by an earlier panic.
    fn lock(&self) -> Result<MutexGuard<'_, Vec<Note>>> {
        self.notes.lock().map_err(|e| Error::Storage(e.to_string()))
    }
}

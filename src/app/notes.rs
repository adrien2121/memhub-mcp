//! Note service: create, read, edit, delete and list notes.

use std::sync::Arc;

use crate::app::embed_worker::{self, Wake};
use crate::app::note_checks::{check_body, check_title};
use crate::app::{chunk, section};
use crate::domain::{EditOp, NewChunk, NewNote, Note, NoteId};
use crate::error::Result;
use crate::ports::embedder::{SharedEmbedder, Tokenizer};
use crate::ports::store::NoteStore;

mod access;

/// Validates input and applies edits on top of a [`NoteStore`].
pub struct NoteService {
    store: Arc<dyn NoteStore>,
    tokenizer: Option<Arc<dyn Tokenizer>>,
    wake: Option<Wake>,
}

impl NoteService {
    /// Creates the service, keeping only the embedder's tokenizer (taken under a brief embedder
    /// lock) so writes can chunk text without holding the embedder.
    ///
    /// # Arguments
    /// * `store` - Note storage shared with the other services.
    /// * `embedder` - Configured embedder; `None` stores notes without chunks until a re-embed.
    /// * `wake` - Signals the background drainer after each committed write; `None` skips it.
    ///
    /// # Returns
    /// The service.
    pub fn new(store: Arc<dyn NoteStore>, embedder: Option<SharedEmbedder>, wake: Option<Wake>) -> Self {
        let tokenizer = embedder.map(|e| embed_worker::lock(&e).tokenizer());
        Self { store, tokenizer, wake }
    }

    /// Splits `body` into embedding chunks with the stored tokenizer, without the embedder lock.
    ///
    /// # Arguments
    /// * `body` - Note body about to be stored.
    ///
    /// # Returns
    /// The chunks; empty if there is no embedder.
    ///
    /// # Errors
    /// * [`Error::Embedder`](crate::error::Error::Embedder) - The tokenizer fails.
    fn chunks(&self, body: &str) -> Result<Vec<NewChunk>> {
        let Some(t) = &self.tokenizer else { return Ok(Vec::new()) };
        chunk::split(body, t.as_ref())
    }

    /// Wakes the background drainer, if any, so new chunks are embedded off the request path.
    /// Non-blocking.
    fn embed_pending(&self) {
        if let Some(wake) = &self.wake {
            wake();
        }
    }

    /// Validates and stores a new note with its chunks pending embedding (one store
    /// transaction), then wakes the drainer.
    ///
    /// # Arguments
    /// * `note` - Title, body and tags.
    /// * `overwrite` - Replace the live note with the same title instead of failing.
    ///
    /// # Returns
    /// The stored note with its id and version.
    ///
    /// # Errors
    /// * [`Error::InvalidInput`](crate::error::Error::InvalidInput) - The title is blank or over 200 characters.
    /// * [`Error::TooLarge`](crate::error::Error::TooLarge) - The body exceeds the size limit.
    /// * [`Error::Conflict`](crate::error::Error::Conflict) - The title exists and `overwrite` is false.
    /// * [`Error::Embedder`](crate::error::Error::Embedder) - Tokenising the body fails.
    /// * [`Error::Storage`](crate::error::Error::Storage) - The store fails.
    pub fn write(&self, note: NewNote, overwrite: bool) -> Result<Note> {
        check_title(&note.title)?;
        check_body(&note.body)?;
        let created = self.store.create(&note, &self.chunks(&note.body)?, overwrite)?;
        self.embed_pending();
        Ok(created)
    }

    /// Applies `op` to the note if it is still at `expected_version` (optimistic concurrency).
    /// When the body changes it is re-chunked and the drainer woken; when only tags change the
    /// existing chunks and embeddings are kept.
    ///
    /// # Arguments
    /// * `id` - Note to edit.
    /// * `expected_version` - Version the caller last read.
    /// * `op` - Replace body, append, replace one Markdown section, or set tags.
    ///
    /// # Returns
    /// The edited note with its incremented version.
    ///
    /// # Errors
    /// * [`Error::NotFound`](crate::error::Error::NotFound) - No live note has `id`.
    /// * [`Error::VersionMismatch`](crate::error::Error::VersionMismatch) - The note changed since `expected_version`.
    /// * [`Error::InvalidInput`](crate::error::Error::InvalidInput) - `ReplaceSection` names no existing heading.
    /// * [`Error::TooLarge`](crate::error::Error::TooLarge) - The resulting body exceeds the size limit.
    /// * [`Error::Embedder`](crate::error::Error::Embedder) - Tokenising the new body fails.
    /// * [`Error::Storage`](crate::error::Error::Storage) - The store fails.
    pub fn edit(&self, id: NoteId, expected_version: i64, op: EditOp) -> Result<Note> {
        let note = self.read_id(id)?;
        let (mut body, mut tags) = (note.body.clone(), note.tags);
        match op {
            EditOp::ReplaceBody(text) => body = text,
            EditOp::Append(text) => body.push_str(&text),
            EditOp::ReplaceSection { heading, content } => body = section::replace(&body, &heading, &content)?,
            EditOp::SetTags(new) => tags = new,
        }
        check_body(&body)?;
        if body == note.body {
            return self.store.edit(id, expected_version, &body, &tags, &[]);
        }
        let edited = self.store.edit(id, expected_version, &body, &tags, &self.chunks(&body)?)?;
        self.embed_pending();
        Ok(edited)
    }
}

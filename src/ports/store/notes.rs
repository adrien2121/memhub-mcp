//! Note records: create, read, edit, delete, list and re-chunk notes.

use crate::domain::{ListQuery, NewChunk, NewNote, Note, NoteId, Page, Tag};
use crate::error::Result;

/// Durable note records and their chunk sets; part of [`super::NoteStore`].
pub trait NoteRecords {
    /// Stores a note and its chunks in one transaction; the chunks start with embeddings pending.
    ///
    /// # Arguments
    /// * `note` - Title, body and tags of the new note.
    /// * `chunks` - Chunks of the body, each awaiting an embedding.
    /// * `overwrite` - Replace the live note with the same title instead of failing.
    ///
    /// # Returns
    /// The stored note with its id, version and timestamps.
    ///
    /// # Errors
    /// * [`crate::error::Error::Conflict`] - The title is taken and `overwrite` is false.
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn create(&self, note: &NewNote, chunks: &[NewChunk], overwrite: bool) -> Result<Note>;

    /// Reads a live note by id.
    ///
    /// # Arguments
    /// * `id` - Note id.
    ///
    /// # Returns
    /// The note, or `None` if no live note has this id.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn read_by_id(&self, id: NoteId) -> Result<Option<Note>>;

    /// Reads a live note by title, ignoring case.
    ///
    /// # Arguments
    /// * `title` - Title to look up.
    ///
    /// # Returns
    /// The note, or `None` if no live note has this title.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn read_by_title(&self, title: &str) -> Result<Option<Note>>;

    /// Replaces body, tags and chunks if the stored version still equals `expected_version`
    /// (optimistic concurrency: a concurrent edit makes this one fail instead of being lost).
    ///
    /// # Arguments
    /// * `id` - Note to edit.
    /// * `expected_version` - Version the caller last read.
    /// * `body` - New body.
    /// * `tags` - New full tag set.
    /// * `chunks` - Chunks of the new body; changed ones await an embedding.
    ///
    /// # Returns
    /// The note with its incremented version.
    ///
    /// # Errors
    /// * [`crate::error::Error::NotFound`] - No live note has this id.
    /// * [`crate::error::Error::VersionMismatch`] - The stored version differs.
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn edit(&self, id: NoteId, expected_version: i64, body: &str, tags: &[Tag], chunks: &[NewChunk]) -> Result<Note>;

    /// Deletes a note permanently with its full-text index entry, tags and chunks.
    ///
    /// # Arguments
    /// * `id` - Note to delete.
    ///
    /// # Errors
    /// * [`crate::error::Error::NotFound`] - No live note has this id.
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn delete(&self, id: NoteId) -> Result<()>;

    /// Lists live notes matching the filters, one page at a time.
    ///
    /// # Arguments
    /// * `query` - Filters, page size and the cursor returned with the previous page.
    ///
    /// # Returns
    /// One page of notes and the cursor of the next page, if any.
    ///
    /// # Errors
    /// * [`crate::error::Error::InvalidInput`] - The cursor is malformed.
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn list(&self, query: &ListQuery) -> Result<Page<Note>>;

    /// Replaces a live note's chunks without changing its version; new chunks await embeddings.
    /// Does nothing if the note's body is no longer `body`: the chunks were split from a stale
    /// read, and the edit that changed the body already stored its own chunks.
    ///
    /// # Arguments
    /// * `id` - Note to re-chunk.
    /// * `body` - The body `chunks` were split from.
    /// * `chunks` - The note's new chunks.
    ///
    /// # Errors
    /// * [`crate::error::Error::NotFound`] - No live note has this id.
    /// * [`crate::error::Error::Storage`] - The storage engine fails.
    fn rechunk(&self, id: NoteId, body: &str, chunks: &[NewChunk]) -> Result<()>;
}

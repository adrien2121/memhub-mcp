//! `NoteRecords` impl: note create, read, edit, delete, list and rechunk.

use super::super::{SqliteStore, chunks, list, notes, tier};
use crate::domain::{ListQuery, NewChunk, NewNote, Note, NoteId, Page, Tag};
use crate::error::{Error, Result};
use crate::ports::store::NoteRecords;

impl NoteRecords for SqliteStore {
    /// Creates a note with its chunks in one `IMMEDIATE` write transaction, then, after commit and
    /// while still holding the writer lock, drops the embeddings of any overwritten note from the
    /// in-memory vector tier.
    ///
    /// # Arguments
    /// * `note` - title, body and tags of the new note.
    /// * `chunks` - the body split into chunks; inserted without embeddings.
    /// * `overwrite` - when `true`, a live note with the same title is replaced instead of rejected.
    ///
    /// # Returns
    /// The stored note, with its new id and version.
    ///
    /// # Errors
    /// * [`Error::Conflict`] - a live note has this title and `overwrite` is `false`.
    /// * [`Error::InvalidInput`] - a chunk offset exceeds `i64::MAX`.
    /// * [`Error::Storage`] - the writer or vector-tier lock is poisoned, or SQLite fails.
    fn create(&self, note: &NewNote, chunks: &[NewChunk], overwrite: bool) -> Result<Note> {
        self.write(|tx| notes::create(tx, note, chunks, overwrite), |_, (note, gone)| self.forget(&gone).map(|()| note))
    }

    /// Reads a live (not soft-deleted) note by id on a pooled read connection.
    ///
    /// # Arguments
    /// * `id` - the note to read.
    ///
    /// # Returns
    /// The note with its tags, or `None` if no live note has that id.
    ///
    /// # Errors
    /// * [`Error::Storage`] - the pool lock is poisoned, a connection cannot be opened, or the query fails.
    fn read_by_id(&self, id: NoteId) -> Result<Option<Note>> {
        self.read(|c| notes::by_id(c, id))
    }

    /// Reads a live note by title, case-insensitively, on a pooled read connection.
    ///
    /// # Arguments
    /// * `title` - the title to look up.
    ///
    /// # Returns
    /// The note with its tags, or `None` if no live note has that title.
    ///
    /// # Errors
    /// * [`Error::Storage`] - the pool lock is poisoned, a connection cannot be opened, or the query fails.
    fn read_by_title(&self, title: &str) -> Result<Option<Note>> {
        self.read(|c| notes::by_title(c, title))
    }

    /// Replaces a note's body, tags and chunks if its version matches, in one `IMMEDIATE` write
    /// transaction; after commit, drops embeddings of deleted chunks from the vector tier.
    ///
    /// # Arguments
    /// * `id` - the note to edit.
    /// * `expected_version` - the version the caller last read; guards against lost updates.
    /// * `body` - the new body.
    /// * `tags` - the new tag set; duplicates are stored once.
    /// * `chunks` - the new body split into chunks; unchanged chunk texts keep their embeddings.
    ///
    /// # Returns
    /// The edited note with its version incremented by one.
    ///
    /// # Errors
    /// * [`Error::NotFound`] - no live note has this id.
    /// * [`Error::VersionMismatch`] - the stored version differs from `expected_version`.
    /// * [`Error::InvalidInput`] - a chunk offset exceeds `i64::MAX`.
    /// * [`Error::Storage`] - the writer or vector-tier lock is poisoned, or SQLite fails.
    fn edit(&self, id: NoteId, expected_version: i64, body: &str, tags: &[Tag], chunks: &[NewChunk]) -> Result<Note> {
        self.write(
            |tx| notes::edit(tx, id, expected_version, body, tags, chunks),
            |_, (note, gone)| self.forget(&gone).map(|()| note),
        )
    }

    /// Deletes a live note in one `IMMEDIATE` write transaction, first collecting the ids of its
    /// embedded chunks, then drops those ids from the vector tier after commit.
    ///
    /// # Arguments
    /// * `id` - the note to delete.
    ///
    /// # Errors
    /// * [`Error::NotFound`] - no live note has this id.
    /// * [`Error::Storage`] - the writer or vector-tier lock is poisoned, or SQLite fails.
    fn delete(&self, id: NoteId) -> Result<()> {
        self.write(|tx| Ok((tier::embedded_ids(tx, id)?, notes::delete(tx, id)?)), |_, (gone, ())| self.forget(&gone))
    }

    /// Lists live notes one page at a time on a pooled read connection.
    ///
    /// # Arguments
    /// * `query` - tag and update-time filters, page size and the cursor from the previous page.
    ///
    /// # Returns
    /// One page of notes and the cursor for the next page, if any.
    ///
    /// # Errors
    /// * [`Error::InvalidInput`] - the cursor is malformed or the page size is zero.
    /// * [`Error::Storage`] - the pool lock is poisoned, a connection cannot be opened, or a query fails.
    fn list(&self, query: &ListQuery) -> Result<Page<Note>> {
        self.read(|c| list::list(c, query))
    }

    /// Replaces a live note's chunks in one `IMMEDIATE` write transaction, without touching its
    /// body or version, unless its body is no longer `body` (stale read; skipped); after commit,
    /// drops embeddings of deleted chunks from the vector tier.
    ///
    /// # Arguments
    /// * `id` - the note to rechunk.
    /// * `body` - the body `new` was split from.
    /// * `new` - the new chunks; chunks whose text is unchanged keep their embeddings.
    ///
    /// # Errors
    /// * [`Error::NotFound`] - no live note has this id.
    /// * [`Error::InvalidInput`] - a chunk offset exceeds `i64::MAX`.
    /// * [`Error::Storage`] - the writer or vector-tier lock is poisoned, or SQLite fails.
    fn rechunk(&self, id: NoteId, body: &str, new: &[NewChunk]) -> Result<()> {
        self.write(
            |tx| {
                let note = notes::by_id(tx, id)?.ok_or(Error::NotFound)?;
                if note.body != body {
                    return Ok(Vec::new());
                }
                chunks::replace(tx, id, new)
            },
            |_, gone| self.forget(&gone),
        )
    }
}

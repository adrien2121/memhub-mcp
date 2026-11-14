//! Read, list and delete on [`NoteService`]: thin pass-throughs to the store.

use super::NoteService;
use crate::domain::{ListQuery, Note, NoteId, Page};
use crate::error::{Error, Result};

impl NoteService {
    /// Reads a live note by id.
    ///
    /// # Arguments
    /// * `id` - Note to read.
    ///
    /// # Returns
    /// The note.
    ///
    /// # Errors
    /// * [`Error::NotFound`] - No live note has `id`.
    /// * [`Error::Storage`] - The store fails.
    pub fn read_id(&self, id: NoteId) -> Result<Note> {
        self.store.read_by_id(id)?.ok_or(Error::NotFound)
    }

    /// Reads a live note by title, case-insensitively.
    ///
    /// # Arguments
    /// * `title` - Title to look up.
    ///
    /// # Returns
    /// The note.
    ///
    /// # Errors
    /// * [`Error::NotFound`] - No live note has that title.
    /// * [`Error::Storage`] - The store fails.
    pub fn read_title(&self, title: &str) -> Result<Note> {
        self.store.read_by_title(title)?.ok_or(Error::NotFound)
    }

    /// Deletes a note permanently with its search entry, tags and chunks.
    ///
    /// # Arguments
    /// * `id` - Note to delete.
    ///
    /// # Errors
    /// * [`Error::NotFound`] - No live note has `id`.
    /// * [`Error::Storage`] - The store fails.
    pub fn delete(&self, id: NoteId) -> Result<()> {
        self.store.delete(id)
    }

    /// Lists live notes matching the query's filters, one page at a time.
    ///
    /// # Arguments
    /// * `query` - Filters, page size and the cursor from the previous page.
    ///
    /// # Returns
    /// One page of notes and the cursor for the next, if any.
    ///
    /// # Errors
    /// * [`Error::InvalidInput`] - The cursor is malformed.
    /// * [`Error::Storage`] - The store fails.
    pub fn list(&self, query: ListQuery) -> Result<Page<Note>> {
        self.store.list(&query)
    }
}

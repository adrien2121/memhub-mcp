//! [`NoteRecords`] for [`FakeStore`]: note CRUD over the in-memory vector; chunks are ignored.

use super::FakeStore;
use crate::domain::{ListQuery, NewChunk, NewNote, Note, NoteId, Page, Tag};
use crate::error::{Error, Result};
use crate::ports::store::NoteRecords;

impl NoteRecords for FakeStore {
    /// Appends a note with id one above the highest stored id, version 1 and zero timestamps;
    /// chunks are dropped. A case-insensitive title clash fails, or with `overwrite` removes
    /// the old note first.
    ///
    /// # Arguments
    /// * `note` - Title, body and tags to store.
    /// * `_` - Chunks (ignored).
    /// * `overwrite` - Replace a note with the same title instead of failing.
    ///
    /// # Returns
    /// The stored note.
    ///
    /// # Errors
    /// * [`Error::Conflict`] - The title exists and `overwrite` is false.
    /// * [`Error::Storage`] - The note mutex is poisoned.
    fn create(&self, note: &NewNote, _: &[NewChunk], overwrite: bool) -> Result<Note> {
        let mut notes = self.lock()?;
        let lower = note.title.to_lowercase();
        if let Some(pos) = notes.iter().position(|n| n.title.to_lowercase() == lower) {
            if !overwrite {
                return Err(Error::Conflict { existing_id: notes[pos].id });
            }
            notes.remove(pos);
        }
        let id = NoteId(notes.iter().map(|n| n.id.0).max().unwrap_or(0) + 1);
        let stored = Note {
            id,
            title: note.title.clone(),
            body: note.body.clone(),
            tags: note.tags.clone(),
            created_at: 0,
            updated_at: 0,
            version: 1,
        };
        notes.push(stored.clone());
        Ok(stored)
    }

    /// Finds a note by id.
    ///
    /// # Arguments
    /// * `id` - Note to find.
    ///
    /// # Returns
    /// A clone of the note, or `None` if absent.
    ///
    /// # Errors
    /// * [`Error::Storage`] - The note mutex is poisoned.
    fn read_by_id(&self, id: NoteId) -> Result<Option<Note>> {
        Ok(self.lock()?.iter().find(|n| n.id == id).cloned())
    }

    /// Finds a note by title, case-insensitively.
    ///
    /// # Arguments
    /// * `title` - Title to match.
    ///
    /// # Returns
    /// A clone of the first matching note, or `None`.
    ///
    /// # Errors
    /// * [`Error::Storage`] - The note mutex is poisoned.
    fn read_by_title(&self, title: &str) -> Result<Option<Note>> {
        let lower = title.to_lowercase();
        Ok(self.lock()?.iter().find(|n| n.title.to_lowercase() == lower).cloned())
    }

    /// Replaces body and tags and bumps the version, if the version still equals `expected`;
    /// chunks are ignored and timestamps stay 0.
    ///
    /// # Arguments
    /// * `id` - Note to edit.
    /// * `expected` - Version the caller last read.
    /// * `body` - New body.
    /// * `tags` - New tags.
    /// * `_` - Chunks (ignored).
    ///
    /// # Returns
    /// The edited note.
    ///
    /// # Errors
    /// * [`Error::NotFound`] - No note has `id`.
    /// * [`Error::VersionMismatch`] - The stored version differs from `expected`.
    /// * [`Error::Storage`] - The note mutex is poisoned.
    fn edit(&self, id: NoteId, expected: i64, body: &str, tags: &[Tag], _: &[NewChunk]) -> Result<Note> {
        let mut notes = self.lock()?;
        let note = notes.iter_mut().find(|n| n.id == id).ok_or(Error::NotFound)?;
        if note.version != expected {
            return Err(Error::VersionMismatch { expected, actual: note.version });
        }
        note.body = body.to_owned();
        note.tags = tags.to_vec();
        note.version += 1;
        Ok(note.clone())
    }

    /// Removes a note.
    ///
    /// # Arguments
    /// * `id` - Note to remove.
    ///
    /// # Errors
    /// * [`Error::NotFound`] - No note has `id`.
    /// * [`Error::Storage`] - The note mutex is poisoned.
    fn delete(&self, id: NoteId) -> Result<()> {
        let mut notes = self.lock()?;
        let pos = notes.iter().position(|n| n.id == id).ok_or(Error::NotFound)?;
        notes.remove(pos);
        Ok(())
    }

    /// Lists notes in insertion order, filtered by tag only; never paginates.
    ///
    /// # Arguments
    /// * `query` - Only `tag` and `limit` are honoured.
    ///
    /// # Returns
    /// Up to `limit` notes and no next cursor.
    ///
    /// # Errors
    /// * [`Error::Storage`] - The note mutex is poisoned.
    fn list(&self, query: &ListQuery) -> Result<Page<Note>> {
        let notes = self.lock()?;
        let items = notes.iter().filter(|n| query.tag.as_ref().is_none_or(|t| n.tags.contains(t)));
        Ok(Page { items: items.take(query.limit).cloned().collect(), next_cursor: None })
    }

    /// Does nothing: the fake keeps no chunks.
    ///
    /// # Arguments
    /// * `_` - Note and new chunks (ignored).
    ///
    /// # Errors
    /// Never fails.
    fn rechunk(&self, _: NoteId, _: &str, _: &[NewChunk]) -> Result<()> {
        Ok(())
    }
}

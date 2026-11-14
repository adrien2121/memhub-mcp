//! Plain data types shared by every layer: notes, chunks, tags, hits, pages.

use serde::{Deserialize, Serialize};

/// Largest accepted note body, in bytes.
pub const MAX_BODY: usize = 1_048_576;
/// Search queries are cut to this many bytes before any tokenising.
pub const MAX_QUERY: usize = 2048;

/// Caps a search query at [`MAX_QUERY`] bytes without splitting a UTF-8 character, so tokenisers never see oversized input.
///
/// # Arguments
/// * `query` - Raw user query, any length.
///
/// # Returns
/// The longest prefix of `query` of at most [`MAX_QUERY`] bytes ending on a char boundary (all of `query` if shorter).
pub fn truncate_query(query: &str) -> &str {
    let end = (0..=query.len().min(MAX_QUERY)).rev().find(|&i| query.is_char_boundary(i)).unwrap_or(0);
    query.get(..end).unwrap_or_default()
}

/// Stable note identifier (SQLite rowid).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NoteId(pub i64);

impl std::fmt::Display for NoteId {
    /// Writes the bare rowid number (e.g. `42`), honouring the formatter's width and fill flags.
    ///
    /// # Arguments
    /// * `f` - Destination formatter.
    ///
    /// # Errors
    /// * [`std::fmt::Error`] - The formatter's underlying writer failed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Stable chunk identifier; a new one is issued every time a note's chunks are rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChunkId(pub i64);

/// A note tag (basic-memory folders become tags).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Tag(pub String);

/// One keyword-search result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    /// Matching note.
    pub note_id: NoteId,
    /// Lower is better: raw FTS5 `bm25()` when keyword-only, fused position (from 1) when hybrid.
    pub rank: f64,
    /// FTS5 `snippet()` text.
    pub snippet: String,
}

/// Opaque position after which the next page starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor(pub String);

/// Filters and paging for listing notes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListQuery {
    /// Only notes carrying this tag.
    pub tag: Option<Tag>,
    /// Only notes updated at or after this Unix time.
    pub updated_after: Option<i64>,
    /// Only notes updated before this Unix time.
    pub updated_before: Option<i64>,
    /// Page size.
    pub limit: usize,
    /// Resume after this cursor; `None` starts from the beginning.
    pub cursor: Option<Cursor>,
}

/// One page of results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    /// Items in this page.
    pub items: Vec<T>,
    /// Cursor for the next page, `None` on the last page.
    pub next_cursor: Option<Cursor>,
}

mod note;
mod search;
pub use note::{Chunk, EditOp, NewChunk, NewNote, Note};
pub use search::{EmbedderChange, ReembedStatus, SearchFilter, SearchRequest, SearchResults};

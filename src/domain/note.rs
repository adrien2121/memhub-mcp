//! Note, edit and chunk types.

use serde::{Deserialize, Serialize};

use super::{ChunkId, NoteId, Tag};

/// A stored, live note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Identifier.
    pub id: NoteId,
    /// Unique title (case-insensitive).
    pub title: String,
    /// Markdown body.
    pub body: String,
    /// Tags, sorted.
    pub tags: Vec<Tag>,
    /// Creation time, Unix seconds.
    pub created_at: i64,
    /// Last update time, Unix seconds.
    pub updated_at: i64,
    /// Optimistic-concurrency version, incremented on every edit.
    pub version: i64,
}

/// A note to be created.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewNote {
    /// Unique title.
    pub title: String,
    /// Markdown body.
    pub body: String,
    /// Tags.
    pub tags: Vec<Tag>,
}

/// One edit applied to an existing note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EditOp {
    /// Replace the whole body.
    ReplaceBody(String),
    /// Append text to the end of the body.
    Append(String),
    /// Replace the content under a Markdown heading.
    ReplaceSection {
        /// Heading text, without the leading `#`s.
        heading: String,
        /// New section content.
        content: String,
    },
    /// Replace the tag set.
    SetTags(Vec<Tag>),
}

/// A chunk span computed by the app, before it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewChunk {
    /// Position within the note, from 0.
    pub seq: u32,
    /// Start byte offset in the body.
    pub start: usize,
    /// End byte offset in the body (exclusive).
    pub end: usize,
    /// Chunk text, `body[start..end]`.
    pub text: String,
}

/// A stored chunk of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Identifier.
    pub id: ChunkId,
    /// Owning note.
    pub note_id: NoteId,
    /// Position within the note, from 0.
    pub seq: u32,
    /// Chunk text.
    pub text: String,
}

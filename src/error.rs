//! Crate-wide error type shared by ports, app and adapters.

use crate::domain::NoteId;

/// Every failure the app or an adapter can report.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No live note matches the given id or title.
    #[error("note not found")]
    NotFound,
    /// A note with this title already exists and `overwrite` was false.
    #[error("title already used by note {existing_id}")]
    Conflict {
        /// The note that owns the title.
        existing_id: NoteId,
    },
    /// The note changed since the caller read it.
    #[error("version mismatch: expected {expected}, found {actual}")]
    VersionMismatch {
        /// Version the caller sent.
        expected: i64,
        /// Version currently stored.
        actual: i64,
    },
    /// The request is malformed (bad cursor, unknown section, empty title...).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// A note body exceeds [`crate::domain::MAX_BODY`] bytes.
    #[error("note body is {0} bytes, over the {max} byte limit", max = crate::domain::MAX_BODY)]
    TooLarge(usize),
    /// The storage engine failed.
    #[error("storage: {0}")]
    Storage(String),
    /// The embedder failed or is unavailable.
    #[error("embedder: {0}")]
    Embedder(String),
    /// A filesystem or socket operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Result alias using [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

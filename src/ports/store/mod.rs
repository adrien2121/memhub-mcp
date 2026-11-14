//! `NoteStore` port: durable notes, chunks, keyword index, vectors and model identity.
//!
//! The port is split by responsibility into three traits: [`NoteRecords`] (note CRUD),
//! [`NoteSearch`] (keyword and vector reads) and [`EmbeddingQueue`] (embedding bookkeeping).
//! [`NoteStore`] is implemented automatically for any type that implements all three.

mod embeddings;
mod notes;
mod search;

pub use embeddings::EmbeddingQueue;
pub use notes::NoteRecords;
pub use search::NoteSearch;

/// Synchronous note storage; callers run it on a blocking thread.
///
/// Callers hold it as `dyn NoteStore` (or a `NoteStore` bound) and can call every method of the
/// three traits it combines without importing them.
pub trait NoteStore: NoteRecords + NoteSearch + EmbeddingQueue + Send + Sync {}

impl<T: NoteRecords + NoteSearch + EmbeddingQueue + Send + Sync> NoteStore for T {}

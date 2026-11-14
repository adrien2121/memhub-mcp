//! Application services; depend only on `ports` and `domain`.

pub mod chunk;
#[cfg(test)]
mod drain_tests;
mod drainer;
pub mod embed_worker;
mod embedder_lock;
pub mod embedding;
#[cfg(test)]
mod fake;
pub mod fuse;
pub mod nearest;
mod note_checks;
pub mod notes;
pub mod query;
pub mod reembed;
mod requeue;
pub mod search;
pub mod section;

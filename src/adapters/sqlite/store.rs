//! Store port impls for `SqliteStore`: each call goes to the read pool or the write transaction.
//! One submodule per port trait: note records, search, and the embedding queue.

mod embed_methods;
mod note_methods;
mod search_methods;

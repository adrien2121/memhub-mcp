//! Access to the shared embedder that survives panics.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{MutexGuard, PoisonError};

use crate::error::{Error, Result};
use crate::ports::embedder::{Embedder, SharedEmbedder};

/// Locks the shared embedder mutex, blocking until it is free; if a previous holder panicked
/// (poisoning the mutex) the lock is taken anyway, since the embedder holds no invariant a
/// panic can break.
///
/// # Arguments
/// * `embedder` - The process-wide embedder shared by search, notes and the drainer.
///
/// # Returns
/// The guard; the embedder stays locked until it is dropped.
pub fn lock(embedder: &SharedEmbedder) -> MutexGuard<'_, Box<dyn Embedder>> {
    embedder.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `f` on the locked embedder and catches any panic inside it, so a crashing model
/// (e.g. the ONNX runtime) fails one call instead of the whole server. Holds the lock for the
/// whole of `f`.
///
/// # Arguments
/// * `embedder` - The shared embedder to lock.
/// * `f` - Work to run with exclusive access to the embedder.
///
/// # Returns
/// Whatever `f` returns.
///
/// # Errors
/// * Any error `f` returns, unchanged.
/// * [`Error::Embedder`] - `f` panicked; the message includes the panic text when it is a string.
pub fn guarded<T>(embedder: &SharedEmbedder, f: impl FnOnce(&mut dyn Embedder) -> Result<T>) -> Result<T> {
    let mut e = lock(embedder);
    catch_unwind(AssertUnwindSafe(|| f(e.as_mut()))).unwrap_or_else(|panic| {
        let msg = panic
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        Err(Error::Embedder(format!("embedder panicked: {msg}")))
    })
}

//! Background thread that embeds pending chunks after every write.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::app::embed_worker::drain_until;
use crate::domain::ChunkId;
use crate::error::{Error, Result};
use crate::ports::embedder::SharedEmbedder;
use crate::ports::store::NoteStore;

/// First wait before retrying a drain that hit a storage error; doubles up to [`MAX_BACKOFF`].
const FIRST_BACKOFF: Duration = Duration::from_millis(100);
/// Longest wait between retries of a failing drain.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Wakes the background drainer; cheap and non-blocking.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The background drainer thread; dropping it stops the thread and waits for it to exit.
pub struct Drainer {
    stop: Arc<AtomicBool>,
    wake: Wake,
    thread: Option<JoinHandle<()>>,
}

impl Drainer {
    /// Hands out another handle to wake this drainer; calling it queues one more drain without
    /// blocking (wakes during a drain coalesce).
    ///
    /// # Returns
    /// A cloned [`Wake`] closure, usable from any thread.
    pub fn wake(&self) -> Wake {
        Arc::clone(&self.wake)
    }
}

impl Drop for Drainer {
    /// Stops the background thread: sets the stop flag (a running drain returns before its
    /// next chunk), wakes the thread so it sees the flag, then joins it, blocking until it
    /// exits. A panic in the thread is logged as a warning, not propagated.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        (self.wake)();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::warn!("embedding drainer panicked");
        }
    }
}

/// Starts a background thread that embeds pending chunks once right away and again after every
/// wake. Wakes arriving during a drain coalesce into one more drain. A drain that fails on
/// storage is retried after 100 ms, doubling up to 30 s, without waiting for a wake; a success
/// resets the delay.
///
/// # Arguments
/// * `store` - Store holding the pending chunks; shared with the request handlers.
/// * `embedder` - Shared embedder; locked per chunk, so requests can interleave.
///
/// # Returns
/// The [`Drainer`] owning the thread; drop it to stop and join the thread.
pub fn spawn(store: Arc<dyn NoteStore>, embedder: SharedEmbedder) -> Drainer {
    let (tx, rx) = mpsc::channel::<()>();
    let _ = tx.send(());
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        let mut backoff: Option<Duration> = None;
        loop {
            let woke = match backoff {
                None => rx.recv().is_ok(),
                Some(wait) => !matches!(rx.recv_timeout(wait), Err(mpsc::RecvTimeoutError::Disconnected)),
            };
            if !woke || flag.load(Ordering::SeqCst) {
                return;
            }
            while rx.try_recv().is_ok() {}
            let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                drain_until(store.as_ref(), &embedder, &flag, &mut |_| {})
            }))
            .unwrap_or_else(|_| Err(Error::Embedder("drain panicked".into())));
            match pass {
                Ok(_) => backoff = None,
                Err(err) => {
                    let wait = backoff.map_or(FIRST_BACKOFF, |d| (d * 2).min(MAX_BACKOFF));
                    tracing::warn!("embedding pending chunks failed, retrying in {wait:?}: {err}");
                    backoff = Some(wait);
                }
            }
        }
    });
    let wake: Wake = Arc::new(move || {
        let _ = tx.send(());
    });
    Drainer { stop, wake, thread: Some(thread) }
}

/// Charges one failed attempt to each chunk in `streak` (failures that are not an embedder
/// outage), then clears it.
///
/// # Arguments
/// * `store` - Store receiving the failure counts.
/// * `streak` - Chunks that failed since the last success.
///
/// # Errors
/// * [`Error::Storage`](crate::error::Error::Storage) - Recording a failure fails.
pub(crate) fn charge(store: &dyn NoteStore, streak: &mut Vec<ChunkId>) -> Result<()> {
    for id in streak.drain(..) {
        if store.record_embed_failure(id)? {
            tracing::warn!(chunk = id.0, failed = store.failed_count()?, "gave up embedding chunk");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "drainer_tests.rs"]
mod tests;

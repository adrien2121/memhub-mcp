//! Accept-with-backoff: keeps a failing `accept` (e.g. EMFILE) from spinning a core.

use std::future::Future;
use std::io;
use std::time::Duration;

use tokio::time::Instant;

/// First pause after an accept error; doubles per consecutive error up to [`MAX_BACKOFF`].
const MIN_BACKOFF: Duration = Duration::from_millis(100);
/// Longest pause between retries during an error burst.
const MAX_BACKOFF: Duration = Duration::from_secs(1);

/// Error-burst state kept across accept calls, so a cancelled call (another `select!` branch won)
/// neither re-logs the burst nor retries before the pending deadline.
#[derive(Default)]
pub(super) struct Backoff {
    failures: u32,
    retry_at: Option<Instant>,
}

/// Calls `accept` until it succeeds, sleeping with capped exponential backoff after each error.
/// Logs the first error of a burst only, plus a recovery line once a burst ends.
pub(super) async fn accept_with_backoff<T, F, Fut>(state: &mut Backoff, mut accept: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = io::Result<T>>,
{
    loop {
        if let Some(at) = state.retry_at {
            tokio::time::sleep_until(at).await;
        }
        match accept().await {
            Ok(stream) => {
                if state.failures > 0 {
                    tracing::info!(failures = state.failures, "accept recovered");
                }
                *state = Backoff::default();
                return stream;
            }
            Err(e) => {
                if state.failures == 0 {
                    tracing::warn!("accept failed: {e}; backing off");
                }
                let delay = MIN_BACKOFF.saturating_mul(1 << state.failures.min(4)).min(MAX_BACKOFF);
                state.failures += 1;
                state.retry_at = Some(Instant::now() + delay);
            }
        }
    }
}

#[cfg(test)]
#[path = "accept_tests.rs"]
mod tests;

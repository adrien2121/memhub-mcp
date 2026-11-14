use super::*;
use std::cell::Cell;

fn flaky(calls: &Cell<u32>, fail_until: u32) -> impl Future<Output = io::Result<u32>> + '_ {
    calls.set(calls.get() + 1);
    let n = calls.get();
    async move { if n <= fail_until { Err(io::Error::other("EMFILE")) } else { Ok(n) } }
}

#[tokio::test]
async fn errors_back_off_and_later_success_is_served() {
    let calls = Cell::new(0);
    let mut state = Backoff::default();
    let start = std::time::Instant::now();
    let got = accept_with_backoff(&mut state, || flaky(&calls, 3)).await;
    assert_eq!(got, 4, "the success after the error burst is returned");
    // 100 + 200 + 400 ms of backoff: at most 4 attempts per 700 ms, not a hot spin.
    assert!(start.elapsed() >= Duration::from_millis(700), "elapsed {:?}", start.elapsed());
    assert_eq!(state.failures, 0, "success resets the burst");
}

#[tokio::test]
async fn cancelled_call_keeps_burst_and_deadline() {
    let calls = Cell::new(0);
    let mut state = Backoff::default();
    // Cancel mid-backoff, as when another select! branch wins.
    let cancelled =
        tokio::time::timeout(Duration::from_millis(20), accept_with_backoff(&mut state, || flaky(&calls, 5)));
    assert!(cancelled.await.is_err());
    assert_eq!((calls.get(), state.failures), (1, 1));
    // Re-entry waits out the pending deadline instead of retrying at once.
    let again = tokio::time::timeout(Duration::from_millis(20), accept_with_backoff(&mut state, || flaky(&calls, 5)));
    assert!(again.await.is_err());
    assert_eq!((calls.get(), state.failures), (1, 1), "no immediate retry, burst not restarted");
}

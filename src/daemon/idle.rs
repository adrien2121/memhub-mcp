//! One step of the daemon loop. Pending connections always win over the idle timer, so a client
//! already queued on the socket when the idle deadline passes is served, not dropped.

use std::future::Future;

/// What woke the daemon loop.
pub(super) enum Event<S> {
    /// A client connected.
    Conn(S),
    /// The live-session count changed.
    Changed,
    /// Shutdown signal or idle timeout: leave the loop.
    Exit,
}

/// Waits for the next loop event. `biased` order: shutdown, session-count change, accept, idle;
/// a connection ready in the same poll as the idle timer is accepted, and idle fires only when
/// `idle_armed` holds.
pub(super) async fn next_event<S>(
    accept: impl Future<Output = S>,
    changed: impl Future<Output = ()>,
    shutdown: impl Future<Output = ()>,
    idle: impl Future<Output = ()>,
    idle_armed: bool,
) -> Event<S> {
    tokio::select! {
        biased;
        () = shutdown => Event::Exit,
        () = changed => Event::Changed,
        stream = accept => Event::Conn(stream),
        () = idle, if idle_armed => Event::Exit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connection_ready_at_idle_deadline_is_served() {
        for _ in 0..200 {
            let ev = next_event(async { 7 }, std::future::pending(), std::future::pending(), async {}, true).await;
            assert!(matches!(ev, Event::Conn(7)), "idle exit won over a queued client");
        }
    }
}

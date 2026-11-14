//! Relaying over a link that can't half-close: after stdin EOF, keep copying daemon output
//! until every forwarded request has its reply or the daemon hangs up.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::pending::Pending;

/// Relays bytes both ways, unchanged and flushed at once, until stdin has closed and every
/// forwarded request has its reply, or the daemon hangs up. Complete lines are parsed only to
/// track which requests still await a reply.
///
/// # Arguments
/// * `stdin` - Client input; EOF stops forwarding but not draining.
/// * `to_daemon` - Write half of the daemon link.
/// * `from_daemon` - Read half of the daemon link; EOF ends the relay.
/// * `stdout` - Client output.
///
/// # Returns
/// `Ok(())` after the final flush of `stdout`.
///
/// # Errors
/// * [`std::io::Error`] - Any read, write or flush on the four streams failed.
pub async fn relay(
    mut stdin: impl AsyncRead + Unpin,
    mut to_daemon: impl AsyncWrite + Unpin,
    mut from_daemon: impl AsyncRead + Unpin,
    mut stdout: impl AsyncWrite + Unpin,
) -> std::io::Result<()> {
    let mut pending = Pending::default();
    let (mut up, mut down) = (vec![0; 8192], vec![0; 8192]);
    let mut stdin_open = true;
    while stdin_open || !pending.is_empty() {
        tokio::select! {
            n = stdin.read(&mut up), if stdin_open => match n? {
                0 => stdin_open = false,
                n => {
                    to_daemon.write_all(&up[..n]).await?;
                    to_daemon.flush().await?;
                    pending.client_sent(&up[..n]);
                }
            },
            n = from_daemon.read(&mut down) => match n? {
                0 => break,
                n => {
                    stdout.write_all(&down[..n]).await?;
                    stdout.flush().await?;
                    pending.daemon_sent(&down[..n]);
                }
            },
        }
    }
    stdout.flush().await
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncWriteExt, duplex};

    use super::relay;

    const REQ: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"a\"}\n";

    /// Proves that after stdin hits EOF with one request forwarded, the relay keeps copying
    /// daemon output (a notification, then the reply) to stdout and stops once the reply arrives.
    #[tokio::test]
    async fn stdin_eof_keeps_draining_until_the_pending_reply_arrives() {
        let (mut daemon, from_daemon) = duplex(1024);
        let mut stdout = Vec::new();
        let replies = async {
            daemon.write_all(b"{\"method\":\"note\"}\n").await?;
            daemon.write_all(b"{\"id\":1,\"result\":{}}\n").await
        };
        let (r, w) = tokio::join!(relay(REQ, tokio::io::sink(), from_daemon, &mut stdout), replies);
        assert!(r.is_ok() && w.is_ok());
        assert_eq!(stdout, b"{\"method\":\"note\"}\n{\"id\":1,\"result\":{}}\n");
        drop(daemon);
    }

    /// Proves the relay returns `Ok` with nothing written when the daemon hangs up while a
    /// request is still pending, instead of waiting forever (guarded by a 10 s timeout).
    #[tokio::test]
    async fn daemon_hanging_up_ends_the_drain() {
        let (daemon, from_daemon) = duplex(1024);
        drop(daemon);
        let mut stdout = Vec::new();
        let drained = relay(REQ, tokio::io::sink(), from_daemon, &mut stdout);
        // Hang guard only: without daemon-EOF detection this would block forever.
        let done = tokio::time::timeout(std::time::Duration::from_secs(10), drained).await;
        assert!(matches!(done, Ok(Ok(()))));
        assert!(stdout.is_empty());
    }
}

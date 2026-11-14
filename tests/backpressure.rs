//! Load bounds of the real daemon: concurrent searches under a low fd limit, and a client that
//! floods requests without reading replies.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// A daemon child under `ulimit -n 256`, killed on drop.
struct Daemon {
    child: Child,
    socket: PathBuf,
}

impl Drop for Daemon {
    /// Kills the daemon process and reaps it, so no child outlives the test.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Daemon {
    /// Launches the `memhub-mcp daemon` binary under `sh` with `ulimit -n 256`, using a temp home directory and a per-test socket in `/tmp`.
    ///
    /// Removes any earlier temp directory and socket with the same name, sets `HOME`, the database path, the model directory (which is deliberately not created) and the socket path through environment variables, then polls the socket every 50 ms until it accepts a connection.
    ///
    /// # Arguments
    /// * `name` - label used in the temp directory and socket file names.
    ///
    /// # Returns
    /// A `Daemon` that kills and reaps the child when dropped.
    ///
    /// # Errors
    /// * I/O errors while preparing the directory or spawning the shell.
    /// * `"daemon never listened"` if the socket accepts no connection within 30 seconds.
    fn start(name: &str) -> Res<Self> {
        let dir = std::env::temp_dir().join(format!("memhub-mcp_bp_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let socket = PathBuf::from(format!("/tmp/mmbp_{name}_{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let child = Command::new("sh")
            .args(["-c", "ulimit -n 256 && exec \"$0\" daemon", env!("CARGO_BIN_EXE_memhub-mcp")])
            .env("HOME", &dir)
            .env_remove("XDG_RUNTIME_DIR")
            .env("MEMHUB_MCP_DB_PATH", dir.join("m.db"))
            .env("MEMHUB_MCP_MODEL_DIR", dir.join("no-model"))
            .env("MEMHUB_MCP_SOCKET_PATH", &socket)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let daemon = Self { child, socket };
        let start = Instant::now();
        while UnixStream::connect(&daemon.socket).is_err() {
            if start.elapsed() > Duration::from_secs(30) {
                return Err("daemon never listened".into());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Ok(daemon)
    }

    /// Connects to the daemon's socket and completes the MCP `initialize` handshake.
    ///
    /// Sends an `initialize` request with protocol version `2025-06-18`, reads and discards its reply, then sends the `notifications/initialized` notification.
    ///
    /// # Returns
    /// The write half as a `UnixStream` and a `BufReader` over a clone of that same connection.
    ///
    /// # Errors
    /// * Connection, write, or read failures on the socket.
    fn session(&self) -> Res<(UnixStream, BufReader<UnixStream>)> {
        let mut w = UnixStream::connect(&self.socket)?;
        let mut r = BufReader::new(w.try_clone()?);
        let init = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
        w.write_all(format!("{init}\n").as_bytes())?;
        r.read_line(&mut String::new())?;
        w.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")?;
        Ok((w, r))
    }

    /// Samples the daemon process's thread count, open file descriptor count and resident memory.
    ///
    /// On Linux this reads `/proc/<pid>`. Elsewhere it runs `ps -M` and `lsof`, counting thread rows and lines whose descriptor column starts with a digit. Each call is one sample; callers keep the maximum across samples to get a peak.
    ///
    /// # Returns
    /// `(threads, open_fds, rss_kib)`.
    ///
    /// # Errors
    /// * Failure to read `/proc`, to run `ps` or `lsof`, or to parse the RSS output.
    fn stats(&self) -> Res<(usize, usize, usize)> {
        let pid = self.child.id().to_string();
        // Absolute paths on macOS: running a bare name searches PATH, and each sample then
        // cost about 24 syspolicyd errors, flooding it; absolute paths measured none.
        #[cfg(target_os = "macos")]
        let (ps, lsof) = ("/bin/ps", "/usr/sbin/lsof");
        #[cfg(not(target_os = "macos"))]
        #[cfg_attr(target_os = "linux", allow(unused_variables))]
        let (ps, lsof) = ("ps", "lsof");
        let run = |args: &[&str]| -> Res<String> {
            Ok(String::from_utf8(Command::new(args[0]).args(&args[1..]).output()?.stdout)?)
        };
        let rss = run(&[ps, "-o", "rss=", "-p", &pid])?.trim().parse()?;
        #[cfg(target_os = "linux")]
        let (threads, fds) = (
            std::fs::read_dir(format!("/proc/{pid}/task"))?.count(),
            std::fs::read_dir(format!("/proc/{pid}/fd"))?.count(),
        );
        #[cfg(not(target_os = "linux"))]
        let (threads, fds) = (
            run(&[ps, "-M", "-p", &pid])?.lines().count().saturating_sub(1),
            run(&[lsof, "-p", &pid])?
                .lines()
                .filter(|l| l.split_whitespace().nth(3).is_some_and(|fd| fd.starts_with(|c: char| c.is_ascii_digit())))
                .count(),
        );
        Ok((threads, fds, rss))
    }
}

/// Builds a `search_notes` JSON-RPC request line for request `id`.
///
/// The query is `alpha word<N>` with `N = id % 97`, limited to 50 results. The returned string ends with a newline so it can be written straight to the socket.
///
/// # Arguments
/// * `id` - JSON-RPC request id; also selects the query word.
///
/// # Returns
/// The request line, including the trailing newline.
fn search(id: usize) -> String {
    let query = format!("alpha word{}", id % 97);
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"search_notes","arguments":{{"query":"{query}","limit":50}}}}}}"#
    ) + "\n"
}

/// Returns the element-wise maximum of two `(threads, open_fds, rss_kib)` samples.
///
/// # Arguments
/// * `a` - first sample.
/// * `b` - second sample.
///
/// # Returns
/// A tuple whose each field is the larger of the two inputs' fields.
fn max(a: (usize, usize, usize), b: (usize, usize, usize)) -> (usize, usize, usize) {
    (a.0.max(b.0), a.1.max(b.1), a.2.max(b.2))
}

/// Starts a daemon with a 256-descriptor limit, seeds 300 notes, then runs 16 client connections that each send 64 `search_notes` requests at the same time.
///
/// Each connection has a writer thread and a reader thread, so requests and replies flow concurrently. The test samples the daemon's threads, open descriptors and RSS while the clients run. It asserts that all 1,024 replies arrived, none reports an error, and the peaks stay below 64 threads, 96 descriptors and 256 MiB RSS.
///
/// # Errors
/// * Setup, socket, or thread failures, propagated with `?`.
///
/// # Panics
/// * If a seed note write fails, a client thread panics, or any assertion fails.
#[test]
fn concurrent_searches_under_a_low_fd_limit_all_succeed_with_bounded_resources() -> Res<()> {
    let daemon = Daemon::start("fds")?;
    let (mut w, mut r) = daemon.session()?;
    let body: String = (0..200).map(|i| format!("alpha word{} ", i % 97)).collect();
    for i in 0..300 {
        let note = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"tools/call","params":{{"name":"write_note","arguments":{{"title":"n{i}","body":"{body}"}}}}}}"#,
            i + 1
        );
        w.write_all(format!("{note}\n").as_bytes())?;
        let mut line = String::new();
        r.read_line(&mut line)?;
        assert!(!line.contains("\"isError\":true"), "seed failed: {line}");
    }
    const CONNS: usize = 16;
    const PER: usize = 64;
    let failures = Arc::new(AtomicUsize::new(0));
    let replies = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for c in 0..CONNS {
        let (mut w, mut r) = daemon.session()?;
        handles.push(thread::spawn(move || -> Res<()> {
            for i in 0..PER {
                w.write_all(search(1 + c * PER + i).as_bytes())?;
            }
            Ok(())
        }));
        let (failures, replies) = (Arc::clone(&failures), Arc::clone(&replies));
        handles.push(thread::spawn(move || -> Res<()> {
            for _ in 0..PER {
                let mut line = String::new();
                if r.read_line(&mut line)? == 0 {
                    return Err("daemon closed the session".into());
                }
                if line.contains("\"isError\":true") || line.contains("\"error\"") {
                    failures.fetch_add(1, Ordering::SeqCst);
                }
                replies.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }));
    }
    let mut peak = daemon.stats()?;
    while !handles.iter().all(thread::JoinHandle::is_finished) {
        peak = max(peak, daemon.stats()?);
    }
    for h in handles {
        h.join().map_err(|_| "client thread panicked")??;
    }
    assert_eq!(replies.load(Ordering::SeqCst), CONNS * PER);
    assert_eq!(failures.load(Ordering::SeqCst), 0, "searches failed under the fd limit");
    let (threads, fds, rss) = peak;
    assert!(threads < 64, "daemon peaked at {threads} threads");
    assert!(fds < 96, "daemon peaked at {fds} open fds");
    assert!(rss < 256 * 1024, "daemon peaked at {rss} KiB RSS");
    Ok(())
}

/// Checks that the daemon stops reading from a client that floods requests without reading replies, and that it answers every request once the client drains them.
///
/// A writer thread sends 20,000 searches while the main thread watches how many writes were accepted. The test requires the writer to block (the accepted count stops changing for 2 seconds while the writer is still running), fewer than 5,000 requests accepted, daemon peaks below 64 threads, 64 open descriptors and 128 MiB RSS, and then all 20,000 replies to be readable with no error.
///
/// # Errors
/// * Socket, thread, or sampling failures, propagated with `?`.
///
/// # Panics
/// * If the writer finishes, the daemon accepts 5,000 or more requests, a peak exceeds its limit, or a reply reports an error.
#[test]
fn a_client_that_never_reads_replies_stalls_after_a_bounded_number_of_requests() -> Res<()> {
    const SENT: usize = 20_000;
    let daemon = Daemon::start("flood")?;
    let (mut w, mut r) = daemon.session()?;
    let written = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&written);
    let writer = thread::spawn(move || -> Res<()> {
        for id in 1..=SENT {
            w.write_all(search(id).as_bytes())?;
            counter.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    });
    let (mut last, mut since, start) = (usize::MAX, Instant::now(), Instant::now());
    let mut peak = daemon.stats()?;
    while since.elapsed() < Duration::from_secs(2) && start.elapsed() < Duration::from_secs(60) {
        peak = max(peak, daemon.stats()?);
        let now = written.load(Ordering::SeqCst);
        if now != last {
            (last, since) = (now, Instant::now());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let accepted = written.load(Ordering::SeqCst);
    assert!(!writer.is_finished(), "writer stopped without blocking: {accepted} written");
    assert!(accepted < SENT / 4, "daemon kept reading: {accepted} of {SENT} requests accepted");
    let (threads, fds, rss) = peak;
    assert!(threads < 64, "daemon peaked at {threads} threads");
    assert!(fds < 64, "daemon peaked at {fds} open fds");
    assert!(rss < 128 * 1024, "daemon peaked at {rss} KiB RSS");

    // Reading the replies resumes the session: every request is answered.
    r.get_ref().set_read_timeout(Some(Duration::from_secs(30)))?;
    for _ in 0..SENT {
        let mut line = String::new();
        assert!(r.read_line(&mut line)? > 0, "session closed while draining");
        assert!(!line.contains("\"isError\":true"), "search failed: {line}");
    }
    writer.join().map_err(|_| "writer panicked")??;
    Ok(())
}

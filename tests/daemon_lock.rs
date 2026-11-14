//! Single-instance guards: `import` and `reembed` refuse while the daemon's database lock is
//! held, and a daemon never takes over a socket another daemon serves.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use memhub_mcp::cli::{import, reembed};
use memhub_mcp::config::{Config, EmbedderConfig, FtsTokenizer};
use memhub_mcp::daemon;
use memhub_mcp::error::{Error, Result};

/// Creates an empty temp directory named `memhub-mcp_<name>_<pid>`, removing any earlier directory with that name.
///
/// # Arguments
/// * `name` - label for the directory name.
///
/// # Returns
/// The path of the new, empty directory.
///
/// # Errors
/// * I/O errors creating the directory.
fn fresh_dir(name: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Builds a `Config` whose database, socket and model paths all sit in `dir`.
///
/// The model path points at a `no-model` directory that does not exist, so no embedder can load. The socket is `m.sock`, the thread count is 1, and `idle_exit_secs` is `Some(0)`.
///
/// # Arguments
/// * `dir` - directory that will hold `m.db` and `m.sock`.
///
/// # Returns
/// The configuration.
fn config(dir: &Path) -> Config {
    Config {
        db_path: dir.join("m.db"),
        embedder: EmbedderConfig::defaults(&dir.join("no-model")),
        socket_path: dir.join("m.sock"),
        threads: 1,
        exact_max_chunks: 10_000,
        idle_exit_secs: Some(0),
        fts_tokenizer: FtsTokenizer::Porter,
    }
}

/// Reports whether a job result is the `ResourceBusy` I/O error that a job returns when the daemon holds the database lock.
///
/// # Arguments
/// * `result` - outcome to inspect.
///
/// # Returns
/// `true` only for `Err(Error::Io)` whose kind is `ResourceBusy`.
fn busy(result: &Result<()>) -> bool {
    matches!(result, Err(Error::Io(e)) if e.kind() == ErrorKind::ResourceBusy)
}

/// Checks that `reembed` and `import` refuse to run while the daemon holds its lock, and that `import` works once the lock is released.
///
/// Creates a notes directory with one Markdown file and takes `daemon::lock`. It asserts both jobs fail with `ResourceBusy` and that the database file was not created. After the lock is dropped, `import` must succeed and create the database, and `reembed` must fail with `Error::Embedder` because the model directory is missing.
///
/// # Errors
/// * Setup or job errors, propagated with `?`.
///
/// # Panics
/// * If any assertion fails.
#[test]
fn import_and_reembed_refuse_while_the_daemon_lock_is_held() -> Result<()> {
    let dir = fresh_dir("lock")?;
    std::fs::create_dir_all(dir.join("notes"))?;
    std::fs::write(dir.join("notes/a.md"), "---\ntitle: Locked\n---\nbody\n")?;
    let config = config(&dir);

    let held = daemon::lock(&config)?;
    assert!(busy(&reembed::run(&config)), "reembed ran beside the daemon");
    assert!(busy(&import::run(&config, &dir.join("notes"))), "import ran beside the daemon");
    assert!(!config.db_path.exists(), "a refused job opened the database");
    drop(held);

    import::run(&config, &dir.join("notes"))?;
    assert!(config.db_path.exists(), "import wrote nothing");
    let reembed = reembed::run(&config);
    assert!(matches!(reembed, Err(Error::Embedder(_))), "reembed must stop at the missing model: {reembed:?}");
    Ok(())
}

/// Checks that a daemon refuses to start when another listener already serves its socket, and leaves that socket working.
///
/// Binds the socket with `ipc::listen` to stand in for the other daemon, runs `daemon::run` with the same config, and asserts it returns `Error::InvalidInput`. It then asserts a client can still connect to that socket.
///
/// # Errors
/// * Setup errors from directory creation or the listener bind.
///
/// # Panics
/// * If the daemon starts instead of being refused, or the socket stops accepting connections.
#[tokio::test]
async fn daemon_leaves_a_socket_another_daemon_serves() -> Result<()> {
    let dir = fresh_dir("socket")?;
    let config = config(&dir);
    let _other = memhub_mcp::ipc::listen(&config.socket_path)?;

    let refused = daemon::run(config.clone()).await;
    assert!(matches!(refused, Err(Error::InvalidInput(_))), "took over the socket: {refused:?}");
    assert!(memhub_mcp::ipc::connect(&config.socket_path).await.is_ok(), "broke the other daemon's socket");
    Ok(())
}

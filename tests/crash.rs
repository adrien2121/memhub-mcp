//! Crash-durability test: SIGKILL a writer process, then check that committed notes survive and the database passes integrity checks.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::domain::{NewChunk, NewNote, NoteId, SearchFilter, Tag};
use memhub_mcp::ports::store::{NoteRecords, NoteSearch};

const DB_ENV: &str = "MEMHUB_MCP_CRASH_DB";

/// Child process body for `sigkill_keeps_committed_notes`. Does nothing unless `MEMHUB_MCP_CRASH_DB` is set.
///
/// When the variable names a database path, opens the store there and creates notes forever. Each note body contains `crashword`. After `create` returns, the function prints `COMMITTED <id>` to stdout; the parent uses those lines as the record of durable commits. The loop never ends, so the parent must kill the process.
///
/// # Panics
/// * If the store cannot be opened or a note cannot be created.
#[test]
fn crash_child() {
    let Ok(path) = std::env::var(DB_ENV) else {
        return;
    };
    let store = SqliteStore::open_with_tokenizer(&PathBuf::from(path), "porter unicode61").unwrap();
    for i in 0..=u64::MAX {
        let body = format!("crashword body number {i}");
        let chunk = NewChunk { seq: 0, start: 0, end: body.len(), text: body.clone() };
        let note = NewNote { title: format!("c{i}"), body, tags: vec![Tag("crash".into())] };
        let n = store.create(&note, &[chunk], false).unwrap();
        println!("COMMITTED {}", n.id.0);
    }
}

/// Verifies that every note whose commit returned before a SIGKILL survives it, and that the database passes integrity checks.
///
/// Re-runs this test binary as `crash_child` with `MEMHUB_MCP_CRASH_DB` set, reads its stdout until 200 `COMMITTED` lines arrive, kills the child with SIGKILL, then collects any further `COMMITTED` lines it printed before dying. It reopens the database and asserts that each recorded note exists with a body starting `crashword`, that the full-text index finds `crashword`, and that `PRAGMA integrity_check` returns `ok`.
///
/// # Panics
/// * If the child exits early, a line cannot be read or parsed, or any assertion fails.
#[test]
fn sigkill_keeps_committed_notes() {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_crash_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("m.db");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
        .env(DB_ENV, &path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut ids = Vec::new();
    while ids.len() < 200 {
        let line = lines.next().expect("child exited early").unwrap();
        if let Some(id) = line.strip_prefix("COMMITTED ") {
            ids.push(id.parse::<i64>().unwrap());
        }
    }
    child.kill().unwrap(); // SIGKILL on Unix.
    child.wait().unwrap();
    ids.extend(lines.map_while(Result::ok).filter_map(|l| l.strip_prefix("COMMITTED ")?.parse::<i64>().ok()));

    let store = SqliteStore::open_with_tokenizer(&path, "porter unicode61").unwrap();
    for &id in &ids {
        let note = store.read_by_id(NoteId(id)).unwrap().expect("committed note lost");
        assert!(note.body.starts_with("crashword"));
    }
    assert!(!store.fts_search("\"crashword\"", &SearchFilter::default(), 10).unwrap().is_empty());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
    assert_eq!(check, "ok");
}

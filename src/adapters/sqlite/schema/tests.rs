//! WAL checkpoint tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use rusqlite::Connection;

use super::checkpoint_truncate;
use crate::adapters::sqlite::connect;

fn fresh_db(name: &str) -> (std::path::PathBuf, Connection) {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_ckpt_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.db");
    let conn = connect(&path).unwrap();
    conn.execute_batch("PRAGMA wal_autocheckpoint=0; CREATE TABLE t(x); INSERT INTO t VALUES (zeroblob(100000));")
        .unwrap();
    (path, conn)
}

fn wal_len(path: &std::path::Path) -> u64 {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    std::fs::metadata(wal).unwrap().len()
}

#[test]
fn checkpoint_truncate_empties_wal_and_keeps_rows() {
    let (path, conn) = fresh_db("ok");
    assert!(wal_len(&path) > 0);
    checkpoint_truncate(&path).unwrap();
    assert_eq!(wal_len(&path), 0);
    let n: i64 = conn.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn checkpoint_truncate_reports_busy_and_keeps_wal_while_reader_open() {
    let (path, conn) = fresh_db("busy");
    let reader = connect(&path).unwrap();
    reader.execute_batch("BEGIN; SELECT count(*) FROM t;").unwrap();
    conn.execute("INSERT INTO t VALUES (1)", []).unwrap();
    let before = wal_len(&path);
    assert!(checkpoint_truncate(&path).is_err());
    assert_eq!(wal_len(&path), before);
    reader.execute_batch("COMMIT").unwrap();
    checkpoint_truncate(&path).unwrap();
    assert_eq!(wal_len(&path), 0);
}

//! Hard delete purges text, schema migration/guard, write/edit receipts, list limits.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::path::PathBuf;
use std::sync::Arc;

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::app::reembed::ReembedService;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::{ListQuery, NewNote, Tag};
use memhub_mcp::error::Error;
use memhub_mcp::mcp::server::MemoryServer;
use memhub_mcp::ports::store::{NoteRecords, NoteStore};
use rmcp::ServiceExt;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const TOK: &str = "porter unicode61";

/// Returns a path to `m.db` inside a fresh temp directory named `memhub-mcp_dr_<name>_<pid>`, removing any earlier directory with that name.
///
/// # Arguments
/// * `name` - label used in the directory name.
///
/// # Returns
/// The path to `m.db`. The directory exists but holds no database yet.
///
/// # Panics
/// * If the directory cannot be removed or created.
fn db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_dr_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("m.db")
}

/// Builds a `NewNote` with the given title, body and tags, each tag wrapped in `Tag`.
///
/// # Arguments
/// * `title` - note title.
/// * `body` - note body text.
/// * `tags` - tag names, one `Tag` per entry.
///
/// # Returns
/// The assembled `NewNote`.
fn note(title: &str, body: &str, tags: &[&str]) -> NewNote {
    NewNote { title: title.into(), body: body.into(), tags: tags.iter().map(|t| Tag((*t).into())).collect() }
}

/// Runs a SQL query that returns one integer in its first column and returns that integer.
///
/// # Arguments
/// * `c` - connection to query.
/// * `sql` - statement whose first column is an integer.
///
/// # Returns
/// The first column of the first row, as `i64`.
///
/// # Panics
/// * If the query fails or returns no row.
fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// Checks that hard-deleting a note removes its text from `notes`, and its row from the FTS index, while leaving other notes indexed.
///
/// Creates a `Gone` note with the body "zebra stripes" and a `Kept` note, counts FTS docsize rows, deletes `Gone`, and asserts the docsize count drops by one. Then asserts no FTS match for "zebra" and no `notes` row whose body contains it. Uses a raw connection so it sees the stored tables directly.
///
/// # Panics
/// * If the store cannot be opened or a write or query fails, or any assertion fails.
#[test]
fn delete_purges_text_and_fts_row() {
    let path = db("purge");
    let s = SqliteStore::open_with_tokenizer(&path, TOK).unwrap();
    let n = s.create(&note("Gone", "zebra stripes", &[]), &[], false).unwrap();
    s.create(&note("Kept", "lion mane", &[]), &[], false).unwrap();
    let raw = Connection::open(&path).unwrap();
    let before = count(&raw, "SELECT count(*) FROM notes_fts_docsize");
    s.delete(n.id).unwrap();
    assert_eq!(count(&raw, "SELECT count(*) FROM notes_fts_docsize"), before - 1);
    assert_eq!(count(&raw, "SELECT count(*) FROM notes_fts WHERE notes_fts MATCH 'zebra'"), 0);
    assert_eq!(count(&raw, "SELECT count(*) FROM notes WHERE body LIKE '%zebra%'"), 0);
}

/// Rewinds a freshly created database to an older schema `version` so the upgrade path can be tested.
/// It always drops the `text_hash` column (added by the schema version 4 migration: SHA-256 of chunk
/// text, so unchanged chunks keep their embedding); below version 3 it also drops `embed_attempts`
/// (added by the schema version 3 migration: failed embedding attempts per chunk); and for version 1
/// it inserts an old-style soft-deleted note, which the schema version 2 migration must purge.
/// Every rewind inserts one live note with an embedded chunk so the version 4 `text_hash` backfill
/// has a row to hash, then sets `PRAGMA user_version`.
///
/// # Arguments
/// * `path` - SQLite database file; created/opened with the current schema first, then altered in place.
/// * `version` - Target schema version to simulate (0..=3).
///
/// # Panics
/// Panics if opening, any SQL statement, or the FTS sanity assertion for the soft-deleted note fails.
fn rewind(path: &std::path::Path, version: u32) {
    drop(SqliteStore::open_with_tokenizer(path, TOK).unwrap());
    let raw = Connection::open(path).unwrap();
    if raw
        .query_row("SELECT count(*) FROM pragma_table_info('chunks') WHERE name = 'text_hash'", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
        == 1
    {
        raw.execute_batch("ALTER TABLE chunks DROP COLUMN text_hash;").unwrap();
    }
    if version < 3 {
        raw.execute_batch("ALTER TABLE chunks DROP COLUMN embed_attempts;").unwrap();
    }
    raw.execute_batch(
        "INSERT INTO notes(title, body, created_at, updated_at, version, deleted_at) \
         VALUES ('live', 'otter', 1, 1, 1, NULL); \
         INSERT INTO chunks(note_id, seq, start, end, text, embedding) \
         VALUES (last_insert_rowid(), 0, 0, 5, 'otter', x'0000803f');",
    )
    .unwrap();
    if version == 1 {
        raw.execute_batch(
            "INSERT INTO notes(title, body, created_at, updated_at, version, deleted_at) \
             VALUES ('old', 'secret walrus', 1, 1, 1, 2);",
        )
        .unwrap();
        assert_eq!(count(&raw, "SELECT count(*) FROM notes_fts WHERE notes_fts MATCH 'walrus'"), 1);
    }
    raw.pragma_update(None, "user_version", version).unwrap();
}

/// Checks that databases at schema versions 0 through 3 upgrade to version 4 with correct data.
///
/// For each starting version, builds the database with `rewind` (version 0 skips the rewind), reopens it through `SqliteStore` to run migrations, then checks on a raw connection: `user_version` is 4, every `text_hash` is the 32-byte SHA-256 of its chunk text, `embed_attempts` exists, no soft-deleted notes remain, the `walrus` note is purged from both the notes and FTS tables, the live `otter` note is searchable only when a row was seeded, and `integrity_check` returns `ok`.
///
/// # Panics
/// * If any open, SQL statement, or assertion fails.
#[test]
fn upgrade_from_every_prior_version() {
    for from in 0..=3u32 {
        let path = db(&format!("upgrade{from}"));
        if from > 0 {
            rewind(&path, from);
        }
        drop(SqliteStore::open_with_tokenizer(&path, TOK).unwrap());
        let raw = Connection::open(&path).unwrap();
        assert_eq!(count(&raw, "PRAGMA user_version"), 4, "from {from}");
        assert_eq!(count(&raw, "SELECT count(*) FROM chunks WHERE length(text_hash) <> 32"), 0, "from {from}");
        let hashes: Vec<(String, Vec<u8>)> = raw
            .prepare("SELECT text, text_hash FROM chunks")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(hashes.len(), usize::from(from > 0), "from {from}");
        for (text, hash) in hashes {
            assert_eq!(hash, Sha256::digest(text.as_bytes()).to_vec(), "from {from}");
        }
        assert_eq!(count(&raw, "SELECT count(*) FROM pragma_table_info('chunks') WHERE name = 'embed_attempts'"), 1);
        assert_eq!(count(&raw, "SELECT count(*) FROM notes WHERE deleted_at IS NOT NULL"), 0, "from {from}");
        assert_eq!(count(&raw, "SELECT count(*) FROM notes_fts WHERE notes_fts MATCH 'walrus'"), 0);
        assert_eq!(count(&raw, "SELECT count(*) FROM notes WHERE body LIKE '%walrus%'"), 0);
        let live = i64::from(from > 0);
        assert_eq!(count(&raw, "SELECT count(*) FROM notes_fts WHERE notes_fts MATCH 'otter'"), live, "from {from}");
        raw.execute("INSERT INTO notes_fts(notes_fts) VALUES ('integrity-check')", []).expect("fts integrity");
        assert_eq!(raw.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)).unwrap(), "ok");
    }
}

/// Checks that opening a database whose `PRAGMA user_version` is 999 fails with a storage error mentioning "newer".
///
/// Creates a current-schema database, sets its version to 999 on a raw connection, then reopens it through `SqliteStore`. Asserts that the open is refused.
///
/// # Panics
/// * If the first open fails, the pragma update fails, the open unexpectedly succeeds, or the error variant or message does not match.
#[test]
fn newer_schema_is_refused() {
    let path = db("newer");
    drop(SqliteStore::open_with_tokenizer(&path, TOK).unwrap());
    Connection::open(&path).unwrap().execute_batch("PRAGMA user_version = 999").unwrap();
    let err = SqliteStore::open_with_tokenizer(&path, TOK).err().expect("must refuse");
    assert!(matches!(&err, Error::Storage(m) if m.contains("newer")), "{err}");
}

/// Checks that `list` rejects `limit: 0`, caps a request for 1000 at 100 items with a next cursor, and returns tags sorted by name.
///
/// Creates 105 notes, each tagged `b` and `a` in that order, then runs the list query twice. Asserts that the invalid limit returns `Error::InvalidInput`, and that the capped page holds 100 notes with `a` before `b` in every tag list.
///
/// # Panics
/// * If the store cannot be opened or a note cannot be created, or any assertion fails.
#[test]
fn list_limit_validated_and_tags_batched() {
    let s = SqliteStore::open_with_tokenizer(&db("limit"), TOK).unwrap();
    for i in 0..105 {
        s.create(&note(&format!("n{i}"), "x", &["b", "a"]), &[], false).unwrap();
    }
    assert!(matches!(s.list(&ListQuery { limit: 0, ..Default::default() }), Err(Error::InvalidInput(_))));
    let page = s.list(&ListQuery { limit: 1000, ..Default::default() }).unwrap();
    assert_eq!(page.items.len(), 100);
    assert!(page.next_cursor.is_some());
    assert!(page.items.iter().all(|n| n.tags == vec![Tag("a".into()), Tag("b".into())]));
}

/// Checks that the `write_note` and `edit_note` MCP tools reply with a receipt holding exactly `id`, `updated_at` and `version`, and that the edit bumps the version from 1 to 2.
///
/// Serves a `MemoryServer` over one end of a `tokio::io::duplex` pipe, completes the MCP `initialize` handshake, then sends one `write_note` and one `edit_note` call, reading each reply before sending the next. Each receipt's JSON body is parsed and its sorted keys and `version` are asserted.
///
/// # Panics
/// * If a reply is missing, is not valid JSON, or has the wrong keys or version.
#[tokio::test(flavor = "multi_thread")]
async fn write_and_edit_return_receipts() {
    let store: Arc<dyn NoteStore> = Arc::new(SqliteStore::open_with_tokenizer(&db("receipt"), TOK).unwrap());
    let notes = Arc::new(NoteService::new(Arc::clone(&store), None, None));
    let search = Arc::new(SearchService::new(Arc::clone(&store), None));
    let server = MemoryServer::new(notes, search, Arc::new(ReembedService::new(store, None, None)));
    let (client, srv) = tokio::io::duplex(1 << 16);
    let (sr, sw) = tokio::io::split(srv);
    tokio::spawn(async move {
        if let Ok(running) = server.serve((sr, sw)).await {
            let _ = running.waiting().await;
        }
    });
    let (cr, mut cw) = tokio::io::split(client);
    let mut lines = BufReader::new(cr).lines();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    cw.write_all(format!("{init}\n").as_bytes()).await.unwrap();
    lines.next_line().await.unwrap();
    let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    cw.write_all(format!("{initialized}\n").as_bytes()).await.unwrap();
    // Sent one at a time: the server handles requests concurrently.
    let calls = [
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"write_note","arguments":{"title":"t","body":"secret body"}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"edit_note","arguments":{"id":1,"expected_version":1,"append":"more"}}}"#,
    ];
    for (call, version) in calls.into_iter().zip([1, 2]) {
        cw.write_all(format!("{call}\n").as_bytes()).await.unwrap();
        let reply = lines.next_line().await.unwrap().unwrap_or_default();
        let v: rmcp::serde_json::Value = rmcp::serde_json::from_str(&reply).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or_default();
        let body: rmcp::serde_json::Value = rmcp::serde_json::from_str(text).expect(&reply);
        let mut keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["id", "updated_at", "version"], "{reply}");
        assert_eq!(body["version"], version);
    }
}

//! SQL-side search filters: tag and update-time.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::path::PathBuf;
use std::sync::Arc;

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::{SearchFilter, SearchRequest, Tag};

/// Returns `m.db` inside a freshly recreated temp directory named after `name` and the process id.
///
/// Removes any previous directory of that name, then creates the new one on disk.
///
/// # Arguments
/// * `name` - test name used in the directory name.
///
/// # Returns
/// The path to `m.db` in that directory.
///
/// # Panics
/// Panics if the directory cannot be created.
fn db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_filter_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("m.db")
}

/// Inserts `(title, body, updated_at, tag)` rows directly into the database in one transaction. FTS triggers index them, and each `Some` tag is linked through `note_tags`.
///
/// # Arguments
/// * `path` - database file to open and write to.
/// * `rows` - `(title, body, updated_at, tag)` tuples; `updated_at` is stored as both `created_at` and `updated_at`.
///
/// # Panics
/// Panics on any SQLite error.
fn seed(path: &PathBuf, rows: &[(String, String, i64, Option<&str>)]) {
    let mut conn = rusqlite::Connection::open(path).unwrap();
    let tx = conn.transaction().unwrap();
    for (title, body, at, tag) in rows {
        tx.execute(
            "INSERT INTO notes(title, body, created_at, updated_at, version) VALUES (?1, ?2, ?3, ?3, 1)",
            rusqlite::params![title, body, at],
        )
        .unwrap();
        if let Some(tag) = tag {
            let id = tx.last_insert_rowid();
            tx.execute("INSERT INTO note_tags(note_id, tag) VALUES (?1, ?2)", rusqlite::params![id, tag]).unwrap();
        }
    }
    tx.commit().unwrap();
}

/// Runs `query` through a keyword-only `SearchService` with `filter` and returns the matching note ids in rank order.
///
/// # Arguments
/// * `store` - store to search.
/// * `query` - search text.
/// * `filter` - search filter to apply.
///
/// # Returns
/// The note id values of the hits, capped at 10.
///
/// # Panics
/// Panics if the search fails.
fn search(store: Arc<SqliteStore>, query: &str, filter: SearchFilter) -> Vec<i64> {
    let req = SearchRequest { query: query.into(), limit: 10, filter };
    SearchService::new(store, None).search(&req).unwrap().hits.iter().map(|h| h.note_id.0).collect()
}

/// Proves a tag filter finds a `rust` note that ranks below the top 500 unfiltered hits. Seeds 600 `rust rust` notes plus one tagged `rare` note with a long body. The tagged note is absent unfiltered, and the tag filter returns only id 601.
#[test]
fn tag_below_top_500_is_found() {
    let path = db("deep");
    let store = Arc::new(SqliteStore::open_with_tokenizer(&path, "porter unicode61").unwrap());
    let mut rows: Vec<_> = (0..600).map(|i| (format!("n{i}"), "rust rust".to_owned(), 1, None)).collect();
    let filler = "padding words ".repeat(200);
    rows.push(("deep".into(), format!("rust {filler}"), 1, Some("rare")));
    seed(&path, &rows);
    let unfiltered = search(store.clone(), "rust", SearchFilter::default());
    assert!(!unfiltered.contains(&601));
    let tag = SearchFilter { tag: Some(Tag("rare".into())), ..Default::default() };
    assert_eq!(search(store, "rust", tag), vec![601]);
}

/// Proves the update-time bounds. `updated_after: 200` includes the boundary (ids 2 and 3), `updated_before: 200` excludes it (id 1 only), and the window 150 to 250 returns id 2.
#[test]
fn date_filter_bounds() {
    let path = db("dates");
    let store = Arc::new(SqliteStore::open_with_tokenizer(&path, "porter unicode61").unwrap());
    let rows: Vec<_> = [100, 200, 300].iter().map(|&t| (format!("t{t}"), "rust".to_owned(), t, None)).collect();
    seed(&path, &rows);
    let sorted = |mut v: Vec<i64>| {
        v.sort_unstable();
        v
    };
    let after = SearchFilter { updated_after: Some(200), ..Default::default() };
    assert_eq!(sorted(search(store.clone(), "rust", after)), vec![2, 3]);
    let before = SearchFilter { updated_before: Some(200), ..Default::default() };
    assert_eq!(search(store.clone(), "rust", before), vec![1]);
    let window = SearchFilter { updated_after: Some(150), updated_before: Some(250), ..Default::default() };
    assert_eq!(search(store, "rust", window), vec![2]);
}

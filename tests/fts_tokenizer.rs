//! FTS tokenizer config: accent folding and rebuild on switch.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::path::PathBuf;

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::domain::{NewNote, SearchFilter};
use memhub_mcp::ports::store::{NoteRecords, NoteSearch};

/// Returns a fresh database path in a per-test, per-process temp directory, wiping any leftover dir.
///
/// # Arguments
/// * `name` - Test-specific directory suffix.
///
/// # Returns
/// Path of `m.db` inside the emptied directory (the file itself is not created).
///
/// # Panics
/// Panics if the directory cannot be created.
fn db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_tok_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("m.db")
}

/// Builds an untagged [`NewNote`]; the tags argument is ignored.
///
/// # Arguments
/// * `title` - Note title.
/// * `body` - Note body.
/// * `_tags` - Unused.
///
/// # Returns
/// The note to create.
fn note(title: &str, body: &str, _tags: &[&str]) -> NewNote {
    NewNote { title: title.into(), body: body.into(), tags: Vec::new() }
}

const UNICODE61: &str = "unicode61 remove_diacritics 2";

/// Proves the `unicode61 remove_diacritics 2` tokenizer folds accents: a French note with
/// "décisions" and "été" is found by the unaccented query "decisions" "ete".
#[test]
fn unicode61_finds_french_accents() {
    let s = SqliteStore::open_with_tokenizer(&db("fr"), UNICODE61).unwrap();
    let n = s.create(&note("Réunion", "décisions prises à l'été", &[]), &[], false).unwrap();
    let hits = s.fts_search("\"decisions\" \"ete\"", &SearchFilter::default(), 10).unwrap();
    assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), vec![n.id]);
}

/// Proves reopening a database with a different FTS tokenizer rebuilds the index: with plain
/// unicode61 the stem "run" no longer matches "running", and reopening with the Porter stemmer
/// makes "run" match again.
#[test]
fn switching_tokenizer_rebuilds_fts() {
    let path = db("retok");
    let n = SqliteStore::open_with_tokenizer(&path, "porter unicode61")
        .unwrap()
        .create(&note("Runs", "running fast", &[]), &[], false)
        .unwrap();
    let s = SqliteStore::open_with_tokenizer(&path, UNICODE61).unwrap();
    assert_eq!(s.fts_search("\"running\"", &SearchFilter::default(), 10).unwrap()[0].0, n.id);
    assert!(s.fts_search("\"run\"", &SearchFilter::default(), 10).unwrap().is_empty());
    drop(s);
    let s = SqliteStore::open_with_tokenizer(&path, "porter unicode61").unwrap();
    assert_eq!(s.fts_search("\"run\"", &SearchFilter::default(), 10).unwrap()[0].0, n.id);
}

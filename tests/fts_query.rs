//! `app::query::build` output is always a valid FTS5 MATCH expression.

use memhub_mcp::app::query::build;
use rusqlite::Connection;

/// Proves `query::build` never emits an invalid FTS5 expression: each adversarial input (FTS
/// operators, punctuation, quotes, column filters, emoji, empty and 100-word strings) that yields a
/// query is run as `MATCH` against an in-memory FTS5 table without an SQLite error.
#[test]
fn adversarial_queries_are_valid_fts5() -> rusqlite::Result<()> {
    let db = Connection::open_in_memory()?;
    db.execute_batch("CREATE VIRTUAL TABLE t USING fts5(body); INSERT INTO t VALUES ('rust AND sqlite notes');")?;
    let hundred = vec!["word"; 100].join(" ");
    let inputs = [
        "?",
        "-",
        "/",
        ",",
        "\"",
        ":",
        "(",
        ")",
        "*",
        "^",
        "AND",
        "OR",
        "NOT",
        "NEAR",
        "? - / , \" : ( ) * ^ AND OR NOT NEAR",
        "a\"b",
        "NEAR(rust sqlite, 2)",
        "body:rust",
        "",
        "🦀 rust 🎉",
        &hundred,
    ];
    for input in inputs {
        if let Some(q) = build(input).map(|f| f.query) {
            let mut stmt = db.prepare("SELECT count(*) FROM t WHERE t MATCH ?1")?;
            let n: i64 = stmt.query_row([&q], |r| r.get(0))?;
            assert!(n >= 0, "{input:?}");
        }
    }
    Ok(())
}

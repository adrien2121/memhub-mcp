//! Forward-only schema migrations, indexed by SQLite `user_version`.

use rusqlite::Connection;

use super::chunk_sync::add_text_hash;

/// One forward migration: plain SQL, or Rust for steps SQLite cannot express.
pub(super) enum Step {
    Sql(&'static str),
    Rust(fn(&Connection) -> rusqlite::Result<()>),
}

/// Migration `i` brings the database from `user_version = i` to `i + 1`.
pub(super) const MIGRATIONS: &[Step] = &[
    Step::Sql(
        r"
CREATE TABLE notes(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL COLLATE NOCASE,
  body TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  version INTEGER NOT NULL,
  deleted_at INTEGER NULL);
CREATE UNIQUE INDEX notes_live_title ON notes(title) WHERE deleted_at IS NULL;
CREATE INDEX notes_updated ON notes(updated_at, id);
CREATE TABLE note_tags(
  note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  tag TEXT NOT NULL,
  PRIMARY KEY(note_id, tag)) WITHOUT ROWID;
CREATE INDEX note_tags_tag ON note_tags(tag, note_id);
CREATE TABLE chunks(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL, start INTEGER NOT NULL, end INTEGER NOT NULL,
  text TEXT NOT NULL, embedding BLOB NULL);
CREATE INDEX chunks_note ON chunks(note_id);
CREATE INDEX chunks_pending ON chunks(id) WHERE embedding IS NULL;
CREATE VIRTUAL TABLE notes_fts USING fts5(title, body, content='notes', content_rowid='id',
  tokenize='porter unicode61');
CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN
  INSERT INTO notes_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
END;
CREATE TRIGGER notes_ad AFTER DELETE ON notes BEGIN
  INSERT INTO notes_fts(notes_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
END;
CREATE TRIGGER notes_au AFTER UPDATE OF title, body ON notes BEGIN
  INSERT INTO notes_fts(notes_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
  INSERT INTO notes_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
END;
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
",
    ),
    Step::Sql("DELETE FROM notes WHERE deleted_at IS NOT NULL;"),
    Step::Sql("ALTER TABLE chunks ADD COLUMN embed_attempts INTEGER NOT NULL DEFAULT 0;"),
    Step::Rust(add_text_hash),
];

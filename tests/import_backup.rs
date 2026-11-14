//! `import` reads basic-memory Markdown; `backup` copies the database.
mod common;

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::cli::{backup, import};
use memhub_mcp::domain::{ListQuery, Tag};
use memhub_mcp::error::Result;
use memhub_mcp::ports::store::{EmbeddingQueue, NoteStore};

const ALPHA: &str =
    "---\ntitle: Alpha\ntype: note\npermalink: main/alpha\ntags:\n- one\n- two\n---\n\n# Alpha\n\nSee [[Beta]].\n";
const BETA: &str = "---\ntitle: Beta\npermalink: main/sub/beta\n---\n\n## Beta\nbody\n";
const DUP: &str = "---\ntitle: Alpha\n---\nother\n";

/// Counts notes in `store` (up to 100) via the list API.
///
/// # Arguments
/// * `store` - Store to list.
///
/// # Returns
/// Number of listed notes.
///
/// # Errors
/// Any store list error.
fn count(store: &dyn NoteStore) -> Result<usize> {
    Ok(store.list(&ListQuery { limit: 100, ..ListQuery::default() })?.items.len())
}

/// Proves Markdown import and backup end to end: two basic-memory files import with front matter
/// turned into tags (`type:`, `permalink:`, `folder:`) and bodies stripped of front matter, a
/// duplicate title and an oversized body are skipped, chunks stay pending, and `backup` copies the
/// database (same note count) but refuses to overwrite an existing file.
#[test]
fn import_counts_titles_tags_body_then_backup() -> Result<()> {
    let store = common::store("import")?;
    let root = std::env::temp_dir().join(format!("memhub-mcp_import_src_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub/deep"))?;
    std::fs::write(root.join("alpha.md"), ALPHA)?;
    std::fs::write(root.join("sub/deep/beta.md"), BETA)?;
    std::fs::write(root.join("zz-dup.md"), DUP)?;
    std::fs::write(root.join("huge.md"), "x".repeat(memhub_mcp::domain::MAX_BODY + 1))?;
    let (embedder, _) = common::embedder("fake", 64);
    let service = NoteService::new(store.clone(), Some(embedder), None);

    let report = import::import(&service, &root)?;
    assert_eq!(report, import::Report { imported: 2, skipped: 2, failed: 0 }, "duplicate and oversized skipped");
    assert!(service.read_title("huge").is_err());

    let alpha = service.read_title("Alpha")?;
    assert_eq!(alpha.body, "# Alpha\n\nSee [[Beta]].\n");
    let want = ["one", "permalink:main/alpha", "two", "type:note"];
    assert_eq!(alpha.tags, want.map(|t| Tag(t.into())).to_vec());
    let beta = service.read_title("Beta")?;
    assert_eq!(beta.body, "## Beta\nbody\n");
    assert!(beta.tags.contains(&Tag("folder:sub/deep".into())));
    assert!(store.pending_count()? > 0, "chunks wait for the drainer");

    let dest = root.join("copy.db");
    backup::backup(&store_path("import"), &dest)?;
    assert!(backup::backup(&store_path("import"), &dest).is_err(), "refuses an existing path");
    let copy = SqliteStore::open_with_tokenizer(&dest, "porter unicode61")?;
    assert_eq!(count(&copy)?, count(store.as_ref())?);
    assert_eq!(count(&copy)?, 2);
    Ok(())
}

/// Returns the database path that `common::store` uses for `name` in this process.
///
/// # Arguments
/// * `name` - Test name passed to `common::store`.
///
/// # Returns
/// Path of that store's `m.db`.
fn store_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("memhub-mcp_{name}_{}", std::process::id())).join("m.db")
}

/// Regression: CRLF files keep their frontmatter (title, tags) and lose it from the body.
#[test]
fn crlf_frontmatter_is_parsed() {
    let text = "---\r\ntitle: Gamma\r\ntags: [a, b]\r\n---\r\n\r\nbody\r\n";
    let note = memhub_mcp::adapters::markdown::parse(text, "stem", None);
    assert_eq!(note.title, "Gamma");
    assert_eq!(note.tags, [Tag("a".into()), Tag("b".into())]);
    assert_eq!(note.body, "body\r\n");
}

/// Parses `text` with stem `stem` and no folder.
///
/// # Arguments
/// * `text` - File contents.
///
/// # Returns
/// The parsed note.
fn parse(text: &str) -> memhub_mcp::domain::NewNote {
    memhub_mcp::adapters::markdown::parse(text, "stem", None)
}

/// Regression: a leading `---` thematic break that is not frontmatter keeps the whole text.
#[test]
fn thematic_break_is_not_frontmatter() {
    let text = "---\nMeeting notes: kickoff\nAgreed on scope.\n---\nNext steps follow.";
    let note = parse(text);
    assert_eq!(note.title, "stem");
    assert_eq!(note.body, text);
    let bare = "---\n-\n---\nrest";
    assert_eq!(parse(bare).body, bare, "a bare `-` is not a list item");
}

/// Regression: a UTF-8 BOM before the opening fence does not disable frontmatter.
#[test]
fn bom_frontmatter_is_parsed() {
    let note = parse("\u{feff}---\ntitle: Bom\ntags: [x]\n---\nbody\n");
    assert_eq!((note.title.as_str(), note.body.as_str()), ("Bom", "body\n"));
    assert_eq!(note.tags, [Tag("x".into())]);
}

/// Regression: fence lines may carry trailing spaces.
#[test]
fn padded_fences_are_recognized() {
    let note = parse("--- \ntitle: T\n---  \nbody\n");
    assert_eq!((note.title.as_str(), note.body.as_str()), ("T", "body\n"));
}

/// Regression: quoted inline tags keep their commas.
#[test]
fn quoted_tags_keep_commas() {
    let note = parse("---\ntitle: T\ntags: [\"a,b\", c]\n---\nbody\n");
    assert_eq!(note.tags, [Tag("a,b".into()), Tag("c".into())]);
}

/// Creates a fresh import root named after `name` under the temp dir.
///
/// # Arguments
/// * `name` - Unique test name.
///
/// # Returns
/// The empty directory.
///
/// # Errors
/// Any filesystem error.
#[cfg(unix)]
fn fresh_root(name: &str) -> Result<std::path::PathBuf> {
    let root = std::env::temp_dir().join(format!("memhub-mcp_{name}_src_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

/// Regression: symlinked directories (here a loop) are not followed, so the import finishes.
#[cfg(unix)]
#[test]
fn symlink_loop_import_finishes() -> Result<()> {
    let root = fresh_root("symloop")?;
    std::fs::write(root.join("a.md"), "body\n")?;
    std::os::unix::fs::symlink(".", root.join("l1"))?;
    std::os::unix::fs::symlink(".", root.join("l2"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    let walk_root = root.clone();
    std::thread::spawn(move || {
        let _ = tx.send(memhub_mcp::adapters::markdown::walk(&walk_root).map(|w| w.files));
    });
    let files = rx.recv_timeout(std::time::Duration::from_secs(5)).map_err(std::io::Error::other)??;
    assert_eq!(files, [root.join("a.md")]);
    Ok(())
}

/// Regression: an unreadable subdirectory counts as one failure; the rest still imports.
#[cfg(unix)]
#[test]
fn unreadable_subdir_is_skipped() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let store = common::store("unreadable")?;
    let root = fresh_root("unreadable")?;
    std::fs::write(root.join("a.md"), "body\n")?;
    std::fs::create_dir(root.join("locked"))?;
    std::fs::write(root.join("locked/b.md"), "body\n")?;
    std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o000))?;
    let service = NoteService::new(store, None, None);
    let report = import::import(&service, &root);
    std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o755))?;
    assert_eq!(report?, import::Report { imported: 1, skipped: 0, failed: 1 });
    Ok(())
}

/// Regression: backing up a database path that does not exist fails and creates nothing.
#[test]
fn backup_of_missing_db_fails() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_backup_missing_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let (db, dest) = (dir.join("absent.db"), dir.join("copy.db"));
    assert!(backup::backup(&db, &dest).is_err(), "missing database reported as backed up");
    assert!(!db.exists() && !dest.exists());
    Ok(())
}

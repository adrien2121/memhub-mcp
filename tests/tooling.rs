//! Tooling checks: file size limit and `app/` dependency rule.

use std::fs;
use std::path::{Path, PathBuf};

/// Recursively collects every `.rs` file under `dir` into `out`.
///
/// Walks subdirectories with `fs::read_dir`. Unreadable entries are skipped silently, and a missing directory yields nothing, so no error is raised.
///
/// # Arguments
/// * `dir` - directory to walk.
/// * `out` - vector that receives each file path found.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Returns every `.rs` file under `rel`, resolved from the crate's `CARGO_MANIFEST_DIR`.
///
/// # Arguments
/// * `rel` - path relative to the crate root, such as `src`.
///
/// # Returns
/// The `.rs` file paths found under `rel`.
///
/// # Panics
/// Panics if no `.rs` file exists under `rel`, so a wrong path cannot pass vacuously.
fn files_under(rel: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join(rel), &mut out);
    assert!(!out.is_empty(), "no .rs files under {rel}");
    out
}

/// Proves no source file under `src/` exceeds 150 lines. Every file over the limit is collected with its line count, and the list must be empty.
#[test]
fn no_source_file_over_150_lines() {
    let long: Vec<_> = files_under("src")
        .into_iter()
        .filter_map(|p| {
            let n = fs::read_to_string(&p).ok()?.lines().count();
            (n > 150).then(|| format!("{}: {n}", p.display()))
        })
        .collect();
    assert!(long.is_empty(), "files over 150 lines: {long:?}");
}

/// Proves `src/app` imports no adapter crate or module. It scans the text of each file for `rusqlite`, `ort::`, `rmcp`, `turbovec`, `tokenizers`, `adapters`, and `vectors`, and the hit list must be empty.
#[test]
fn app_uses_no_adapter_crates() {
    const BANNED: [&str; 7] = ["rusqlite", "ort::", "rmcp", "turbovec", "tokenizers", "adapters", "vectors"];
    let mut hits = Vec::new();
    for path in files_under("src/app") {
        let text = fs::read_to_string(&path).unwrap_or_default();
        for word in BANNED {
            if text.contains(word) {
                hits.push(format!("{}: {word}", path.display()));
            }
        }
    }
    assert!(hits.is_empty(), "app/ imports forbidden crates: {hits:?}");
}

//! Reads basic-memory Markdown files: YAML-ish frontmatter plus a verbatim body.

mod parse;

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::domain::{MAX_BODY, NewNote};
use crate::error::{Error, Result};

pub use parse::parse;

/// One parsed source file.
pub struct SourceNote {
    /// File path relative to the import root.
    pub path: PathBuf,
    /// Note to store; tags include `folder:<dir>` and `permalink:<p>` when known.
    pub note: NewNote,
}

/// Result of [`walk`].
pub struct Walk {
    /// Markdown files found (prefixed with `root`), sorted lexicographically.
    pub files: Vec<PathBuf>,
    /// Subdirectories or entries that couldn't be read; each was logged on stderr and skipped.
    pub failed: usize,
}

/// Lists every `.md` file under `root` recursively, skipping any file or directory whose name
/// starts with `.` and never following symlinked directories (so link loops can't recurse);
/// used by the `import` CLI subcommand (bulk-loads basic-memory Markdown notes into the
/// database) to find the files to read.
///
/// # Arguments
/// * `root` - Directory to scan.
///
/// # Returns
/// The files found and how many unreadable subdirectories or entries were skipped.
///
/// # Errors
/// * [`Error::Io`] - `root` itself can't be read.
pub fn walk(root: &Path) -> Result<Walk> {
    let mut walk = Walk { files: Vec::new(), failed: 0 };
    let mut stack = vec![(root.to_path_buf(), std::fs::read_dir(root)?)];
    while let Some((dir, entries)) = stack.pop() {
        for entry in entries {
            let (path, kind) = match entry.and_then(|e| e.file_type().map(|t| (e.path(), t))) {
                Ok(entry) => entry,
                Err(e) => {
                    eprintln!("failed {}: {e}", dir.display());
                    walk.failed += 1;
                    continue;
                }
            };
            let hidden = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
            if hidden {
                continue;
            }
            if kind.is_dir() {
                match std::fs::read_dir(&path) {
                    Ok(sub) => stack.push((path, sub)),
                    Err(e) => {
                        eprintln!("failed {}: {e}", path.display());
                        walk.failed += 1;
                    }
                }
            } else if path.extension().is_some_and(|e| e == "md") {
                walk.files.push(path);
            }
        }
    }
    walk.files.sort();
    Ok(walk)
}

/// Reads the file at `root/rel` and parses it with [`parse`], reading at most [`MAX_BODY`] + 1
/// bytes so an oversized file is detected without loading it whole.
///
/// The title falls back to the file stem and the parent directory of `rel` (if any) becomes a
/// `folder:<dir>` tag.
///
/// # Arguments
/// * `root` - Import root directory.
/// * `rel` - File path relative to `root`; kept verbatim in the result.
///
/// # Returns
/// The relative path and the parsed note.
///
/// # Errors
/// * [`Error::TooLarge`] - The file exceeds [`MAX_BODY`] bytes; carries its size from metadata
///   (or the bytes read if metadata fails).
/// * [`Error::Io`] - The file can't be opened or read, or isn't valid UTF-8 (kind `InvalidData`).
pub fn read(root: &Path, rel: &Path) -> Result<SourceNote> {
    let mut bytes = Vec::new();
    let file = std::fs::File::open(root.join(rel))?;
    file.take(MAX_BODY as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BODY {
        let len =
            std::fs::metadata(root.join(rel)).map_or(bytes.len(), |m| usize::try_from(m.len()).unwrap_or(usize::MAX));
        return Err(Error::TooLarge(len));
    }
    let text = String::from_utf8(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let stem = rel.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = rel.parent().map(|p| p.to_string_lossy().into_owned()).filter(|p| !p.is_empty());
    Ok(SourceNote { path: rel.to_path_buf(), note: parse(&text, &stem, folder.as_deref()) })
}

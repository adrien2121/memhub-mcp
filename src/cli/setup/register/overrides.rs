//! Parses `--path <client>=<path>` overrides for client registration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Client names accepted by `--path`.
pub(crate) const CLIENTS: [&str; 7] = ["claude-code", "codex", "gemini", "cursor", "omp", "claude-desktop", "vscode"];

/// Override path per client name: the CLI executable (claude-code, codex) or the config file.
pub(crate) type Overrides = HashMap<&'static str, PathBuf>;

/// Parses `<client>=<path>` specs, expanding a leading `~/` to `home`.
///
/// # Arguments
/// * `specs` - Raw `--path` values.
/// * `home` - Home directory used for `~/`.
///
/// # Returns
/// The override path for each named client; a later spec for the same client wins.
///
/// # Errors
/// [`Error::InvalidInput`] - A spec has no `=` or names an unknown client; the message lists
/// the valid names.
pub(crate) fn parse(specs: &[String], home: &Path) -> Result<Overrides> {
    let mut out = Overrides::new();
    for spec in specs {
        let invalid = || {
            Error::InvalidInput(format!(
                "invalid --path {spec:?}: expected <client>=<path> with client one of: {}",
                CLIENTS.join(", ")
            ))
        };
        let (name, path) = spec.split_once('=').ok_or_else(invalid)?;
        let name = CLIENTS.into_iter().find(|c| *c == name).ok_or_else(invalid)?;
        out.insert(name, expand(path, home));
    }
    Ok(out)
}

/// Expands a leading `~/` in `path` to `home`.
///
/// # Arguments
/// * `path` - Path as typed by the user.
/// * `home` - Home directory.
///
/// # Returns
/// The expanded path.
pub(crate) fn expand(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(path),
    }
}

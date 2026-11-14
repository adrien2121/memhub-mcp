//! Adds the memhub-mcp entry to a client's JSON config file.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::{NAME, Outcome};

/// Adds `{key: {"memhub-mcp": entry}}` to the JSON config `path` when its parent dir exists,
/// creating the file if missing. An existing file is copied to `<file>.bak` before being
/// replaced atomically; an unparseable file is left untouched.
///
/// # Arguments
/// * `path` - Config file; a missing parent dir means the client is not installed, or a
///   failure when `explicit`.
/// * `explicit` - The user named `path` with `--path`.
/// * `key` - Object holding the servers (`mcpServers` or `servers`).
/// * `entry` - The memhub-mcp server entry.
///
/// # Returns
/// The outcome; failures carry the snippet to add by hand.
pub(super) fn add_json(path: &Path, explicit: bool, key: &str, entry: &Value) -> Outcome {
    if !path.parent().is_some_and(Path::is_dir) {
        return match explicit {
            true => Outcome::Failed(format!("folder of {} does not exist", path.display())),
            false => Outcome::NotFound,
        };
    }
    let fail = |reason: String| {
        let snippet = json!({ key: { NAME: entry } });
        let snippet = serde_json::to_string_pretty(&snippet).unwrap_or_default();
        Outcome::Failed(format!("{reason}\n  add this to {} by hand:\n{snippet}", path.display()))
    };
    let existing = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return fail(format!("can't read {}: {e}", path.display())),
    };
    let mut root = match existing.as_deref().map(str::trim) {
        None | Some("") => Map::new(),
        Some(text) => match serde_json::from_str(text) {
            Ok(Value::Object(map)) => map,
            Ok(_) => return fail(format!("{} is not a JSON object", path.display())),
            Err(e) => return fail(format!("can't parse {}: {e}", path.display())),
        },
    };
    let servers = root.entry(key).or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = servers.as_object_mut() else {
        return fail(format!("\"{key}\" in {} is not an object", path.display()));
    };
    if servers.contains_key(NAME) {
        return Outcome::Already;
    }
    servers.insert(NAME.into(), entry.clone());
    match write(path, existing.is_some(), &Value::Object(root)) {
        Ok(()) => Outcome::Registered,
        Err(e) => fail(format!("can't write {}: {e}", path.display())),
    }
}

/// Writes `value` to `path` via a temp file and rename, first copying an existing file to `.bak`.
///
/// # Arguments
/// * `path` - Destination.
/// * `backup` - Whether `path` exists and must be backed up.
/// * `value` - JSON to write, pretty-printed.
///
/// # Returns
/// `Ok(())` once `path` holds the new content.
///
/// # Errors
/// Any I/O or serialization failure.
fn write(path: &Path, backup: bool, value: &Value) -> std::io::Result<()> {
    if backup {
        fs::copy(path, with_suffix(path, ".bak"))?;
    }
    let tmp = with_suffix(path, ".tmp");
    fs::write(&tmp, serde_json::to_string_pretty(value)? + "\n")?;
    fs::rename(&tmp, path)
}

/// Appends `suffix` to the file name of `path`.
///
/// # Arguments
/// * `path` - Original path.
/// * `suffix` - Text to append, e.g. `.bak`.
///
/// # Returns
/// `path` with `suffix` appended.
pub(super) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves the `.bak` is written before the file is replaced: with `<file>.tmp` blocked by a
    /// directory the write fails, yet `.bak` already holds the original and the file is unchanged.
    #[test]
    fn backup_precedes_write() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("memhub-mcp_bak_order_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("mcp.json.tmp"))?;
        fs::write(dir.join("mcp.json"), "{}")?;
        let outcome = add_json(&dir.join("mcp.json"), false, "mcpServers", &json!({ "command": "x" }));
        assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
        assert_eq!(fs::read_to_string(dir.join("mcp.json"))?, "{}");
        assert_eq!(fs::read_to_string(dir.join("mcp.json.bak"))?, "{}");
        Ok(())
    }
}

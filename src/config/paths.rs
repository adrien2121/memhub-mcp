//! Platform data, config and socket locations.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Resolves the data directory holding the database, models and libraries; performs no I/O beyond reading the home location.
///
/// `~/.local/share/memhub-mcp` on Unix, `%APPDATA%\memhub-mcp` on Windows.
///
/// # Returns
/// The absolute data directory path (not created).
///
/// # Errors
/// * [`Error::InvalidInput`] - The home (Unix) or roaming app data (Windows) directory is unknown.
pub fn data_dir() -> Result<PathBuf> {
    #[cfg(unix)]
    let base = dirs::home_dir().map(|h| h.join(".local/share"));
    #[cfg(windows)]
    let base = dirs::data_dir();
    base.map(|b| b.join("memhub-mcp")).ok_or_else(|| Error::InvalidInput("home directory is unknown".into()))
}

/// Resolves the directory searched for `config.toml`.
///
/// `~/.config/memhub-mcp` on Unix, `%APPDATA%\memhub-mcp` on Windows.
///
/// # Returns
/// The absolute config directory path (not created).
///
/// # Errors
/// * [`Error::InvalidInput`] - The home (Unix) or roaming app data (Windows) directory is unknown.
pub fn config_dir() -> Result<PathBuf> {
    #[cfg(unix)]
    let base = dirs::home_dir().map(|h| h.join(".config"));
    #[cfg(windows)]
    let base = dirs::config_dir();
    base.map(|b| b.join("memhub-mcp")).ok_or_else(|| Error::InvalidInput("home directory is unknown".into()))
}

/// Picks the default daemon socket: `$XDG_RUNTIME_DIR/memhub-mcp.sock` when that variable is set.
///
/// # Arguments
/// * `data` - Data directory used as the fallback location.
///
/// # Returns
/// The runtime-dir socket path, else `daemon.sock` inside `data`.
#[cfg(unix)]
pub(super) fn default_socket(data: &Path) -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| PathBuf::from(d).join("memhub-mcp.sock"))
        .unwrap_or_else(|| data.join("daemon.sock"))
}

/// Builds a named pipe path unique to the data dir, so users and data dirs never share a daemon.
///
/// # Arguments
/// * `data` - Data directory; the first 8 bytes of its SHA-256 become the pipe suffix.
///
/// # Returns
/// `\\.\pipe\memhub-mcp-<16 hex chars>`.
#[cfg(windows)]
pub(super) fn default_socket(data: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(data.as_os_str().as_encoded_bytes());
    let tag: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    PathBuf::from(format!(r"\\.\pipe\memhub-mcp-{tag}"))
}

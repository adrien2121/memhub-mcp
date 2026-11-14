//! Loads `config.toml` plus `MEMHUB_MCP_*` environment overrides into a typed config.

use std::path::{Path, PathBuf};

use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub mod embedder;
mod fts;
mod paths;
pub use embedder::{EmbedderConfig, Pooling};
pub use fts::FtsTokenizer;
pub use paths::{config_dir, data_dir};

/// Daemon and relay settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// SQLite database file.
    pub db_path: PathBuf,
    /// Embedding model settings.
    #[serde(flatten)]
    pub embedder: EmbedderConfig,
    /// Local socket the daemon listens on: a socket file on Unix, a `\\.\pipe\...` path on Windows.
    pub socket_path: PathBuf,
    /// Embedder threads.
    pub threads: usize,
    /// Largest chunk count served by exact vector search.
    pub exact_max_chunks: usize,
    /// Seconds without clients before the daemon exits; `None` means never.
    pub idle_exit_secs: Option<u64>,
    /// FTS5 tokenizer; changing it rebuilds the keyword index on open.
    pub fts_tokenizer: FtsTokenizer,
}

impl Config {
    /// Loads defaults, then `config.toml` in [`config_dir`] (if present), then `MEMHUB_MCP_*` env vars; later layers win.
    ///
    /// # Returns
    /// The merged configuration.
    ///
    /// # Errors
    /// * [`Error::InvalidInput`] - The home directory is unknown, or a value has the wrong type or an unknown variant.
    pub fn load() -> Result<Self> {
        Figment::from(Serialized::defaults(Self::defaults(&data_dir()?)))
            .merge(Toml::file(config_dir()?.join("config.toml")))
            .merge(Env::prefixed("MEMHUB_MCP_"))
            .extract()
            .map_err(|e| Error::InvalidInput(format!("config: {e}")))
    }

    /// Derives the lock file path beside the database, held by the daemon and by `import`/`reembed`.
    ///
    /// # Returns
    /// `db_path` with `.lock` appended (e.g. `memory.db.lock`).
    pub fn lock_path(&self) -> PathBuf {
        let mut path = self.db_path.clone().into_os_string();
        path.push(".lock");
        path.into()
    }

    /// Builds the built-in defaults, the lowest layer of [`Config::load`].
    ///
    /// # Arguments
    /// * `data` - Data directory under which the database, model and socket fallback live.
    ///
    /// # Returns
    /// Defaults: 2 threads, 10 000 exact-search chunks, no idle exit, Porter tokenizer.
    pub(crate) fn defaults(data: &Path) -> Self {
        Self {
            db_path: data.join("memory.db"),
            embedder: EmbedderConfig::defaults(data),
            socket_path: paths::default_socket(data),
            threads: 2,
            exact_max_chunks: 10_000,
            idle_exit_secs: None,
            fts_tokenizer: FtsTokenizer::Porter,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves the built-in defaults are the documented ones (`memory.db` in the data dir, 2 threads, 10 000 exact-search chunks, no idle exit, Porter tokenizer) by building them under `/d`.
    #[test]
    fn defaults_match_handoff() {
        let c = Config::defaults(Path::new("/d"));
        assert_eq!(c.db_path, PathBuf::from("/d/memory.db"));
        assert_eq!((c.threads, c.exact_max_chunks, c.idle_exit_secs), (2, 10_000, None));
        assert_eq!(c.fts_tokenizer.as_sql(), "porter unicode61");
    }
}

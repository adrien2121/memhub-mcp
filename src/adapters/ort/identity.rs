//! Embedder identity stored in `meta`: any change means the stored vectors are stale.

use std::fmt;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::config::{EmbedderConfig, Pooling};
use crate::error::{Error, Result};

/// Everything that changes the vectors a model produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelIdentity {
    /// Model repo and revision.
    pub model_id: String,
    /// SHA-256 of `model.onnx`, hex.
    pub onnx_sha256: String,
    /// SHA-256 of `tokenizer.json`, hex.
    pub tokenizer_sha256: String,
    /// Pooling of the token vectors.
    pub pooling: Pooling,
    /// Query prefix.
    pub query_prefix: String,
    /// Document prefix.
    pub doc_prefix: String,
    /// Largest model input in tokens.
    pub max_tokens: usize,
    /// Vector dimension.
    pub dim: usize,
}

impl ModelIdentity {
    /// Hashes the model files and copies the vector-affecting settings from `cfg`.
    ///
    /// Reads both files fully (streamed, 64 KiB at a time).
    ///
    /// # Arguments
    /// * `cfg` - Embedder settings supplying model id, pooling, prefixes, token limit, dimension.
    /// * `onnx` - Path to `model.onnx`.
    /// * `tokenizer` - Path to `tokenizer.json`.
    ///
    /// # Returns
    /// The identity with hex SHA-256 digests of both files.
    ///
    /// # Errors
    /// * [`Error::Embedder`] - A file cannot be opened or read; the message names the path.
    pub fn compute(cfg: &EmbedderConfig, onnx: &Path, tokenizer: &Path) -> Result<Self> {
        Ok(Self {
            model_id: cfg.model_id.clone(),
            onnx_sha256: sha256(onnx)?,
            tokenizer_sha256: sha256(tokenizer)?,
            pooling: cfg.pooling,
            query_prefix: cfg.query_prefix.clone(),
            doc_prefix: cfg.doc_prefix.clone(),
            max_tokens: cfg.max_tokens,
            dim: cfg.dim,
        })
    }
}

/// `normalize=true` stays in the text, unchanged since the setting was removed (vectors are now
/// always L2-normalised by the app), so existing databases still match.
impl fmt::Display for ModelIdentity {
    /// Writes the one-line `key=value` form stored in the database `meta` table.
    ///
    /// # Arguments
    /// * `f` - Output formatter.
    ///
    /// # Returns
    /// Formatter result.
    ///
    /// # Errors
    /// * [`fmt::Error`] - The formatter fails to write.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "model={} onnx={} tokenizer={} pooling={:?} normalize=true query_prefix={:?} doc_prefix={:?} max_tokens={} dim={}",
            self.model_id,
            self.onnx_sha256,
            self.tokenizer_sha256,
            self.pooling,
            self.query_prefix,
            self.doc_prefix,
            self.max_tokens,
            self.dim
        )
    }
}

/// Streams a file through SHA-256 in 64 KiB reads.
///
/// # Arguments
/// * `path` - File to hash.
///
/// # Returns
/// Lowercase hex digest (64 characters).
///
/// # Errors
/// * [`Error::Embedder`] - The file cannot be opened or read; the message names the path.
fn sha256(path: &Path) -> Result<String> {
    let err = |e: std::io::Error| Error::Embedder(format!("{}: {e}", path.display()));
    let mut file = std::fs::File::open(path).map_err(err)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(err)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

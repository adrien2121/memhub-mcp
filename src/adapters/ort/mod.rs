//! `Embedder` for any ONNX sentence-embedding model through onnxruntime, one text at a time.

mod embed;
mod encode;
mod identity;
#[cfg(test)]
mod limit_tests;
mod pool;
mod session;
#[cfg(test)]
mod tests;
mod tokenizer;

pub use identity::ModelIdentity;

use std::sync::Arc;

use ort::session::Session;
use tokenizers::Tokenizer;

use self::tokenizer::OrtTokenizer;

use crate::config::{Config, EmbedderConfig};
use crate::error::{Error, Result};

/// onnxruntime session plus tokenizer, configured by [`EmbedderConfig`].
pub struct OrtEmbedder {
    session: Session,
    tokenizer: Arc<OrtTokenizer>,
    cfg: EmbedderConfig,
    identity: ModelIdentity,
    /// Model takes `attention_mask`.
    use_mask: bool,
    /// Model takes `token_type_ids`.
    use_type_ids: bool,
}

/// Wraps any displayable onnxruntime or tokenizer failure as an embedder error.
///
/// # Arguments
/// * `e` - The failure; only its `Display` text is kept.
///
/// # Returns
/// [`Error::Embedder`] carrying that text.
fn err(e: impl std::fmt::Display) -> Error {
    Error::Embedder(e.to_string())
}

impl OrtEmbedder {
    /// Hashes the model files, loads the tokenizer, the onnxruntime dylib and the model, and
    /// checks the model's output dimension against the configuration.
    ///
    /// Reads `model.onnx` and `tokenizer.json` from `model_dir`. `OrtTokenizer::new` builds the
    /// tokenizer and its token limits before the model is opened.
    ///
    /// # Arguments
    /// * `config` - App configuration; uses `embedder` settings and `threads` (onnxruntime
    ///   intra-op threads).
    ///
    /// # Returns
    /// A ready embedder.
    ///
    /// # Errors
    /// * [`Error::Embedder`] - A model file or the dylib is missing or invalid; the tokenizer
    ///   fails to encode the document prefix (carrying the tokenizer's message); `max_tokens`
    ///   leaves no room for content; the model has no tensor output; or the model declares a
    ///   fixed output dimension different from the configured `dim`.
    pub fn load(config: &Config) -> Result<Self> {
        let cfg = config.embedder.clone();
        let onnx = cfg.model_dir.join("model.onnx");
        let tok_path = cfg.model_dir.join("tokenizer.json");
        let identity = ModelIdentity::compute(&cfg, &onnx, &tok_path)?;
        let inner = Tokenizer::from_file(&tok_path).map_err(err)?;
        let tokenizer = Arc::new(OrtTokenizer::new(inner, cfg.max_tokens, &cfg.doc_prefix)?);

        let session = session::open(&cfg.ort_dylib, &onnx, config.threads)?;
        let has = |n: &str| session.inputs().iter().any(|i| i.name() == n);
        let (use_mask, use_type_ids) = (has("attention_mask"), has("token_type_ids"));
        let model_dim = session
            .outputs()
            .first()
            .and_then(|o| o.dtype().tensor_shape())
            .and_then(|s| s.last().copied())
            .ok_or_else(|| err("model has no tensor output"))?;
        if model_dim > 0 && model_dim as usize != cfg.dim {
            return Err(err(format!("model dim {model_dim} != configured dim {}", cfg.dim)));
        }
        Ok(Self { session, tokenizer, cfg, identity, use_mask, use_type_ids })
    }

    /// Returns the model identity (file hashes and vector-affecting settings) stored in the
    /// database `meta` table to detect stale vectors.
    ///
    /// # Returns
    /// The identity computed at load time.
    pub fn model_identity(&self) -> &ModelIdentity {
        &self.identity
    }
}

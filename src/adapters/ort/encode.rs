//! Text-to-token encoding for [`super::OrtEmbedder`]: instruction prefix, truncation, special tokens.

use tokenizers::Encoding;

use super::OrtEmbedder;
use crate::config::EmbedderConfig;
use crate::domain::truncate_query;
use crate::error::Result;
use crate::ports::embedder::Kind;

impl OrtEmbedder {
    /// Tokenises the instruction prefix plus `text`, cuts it to the input limit, then adds the
    /// model's special tokens (for example `[CLS]` and `[SEP]`).
    ///
    /// Queries are first cut to [`crate::domain::MAX_QUERY`] bytes by [`truncate_query`]. Tokens
    /// past the shared input limit of [`super::tokenizer::OrtTokenizer::encode_input`] are
    /// dropped from the end; chunks from `app::chunk::split` never reach that limit.
    ///
    /// # Arguments
    /// * `text` - Raw query or document text.
    /// * `kind` - Selects the query or document prefix and whether the query byte cut applies.
    ///
    /// # Returns
    /// The final encoding, at most `max_tokens` tokens long, ready to feed the model.
    ///
    /// # Errors
    /// * [`crate::error::Error::Embedder`] - The tokenizer fails to encode or post-process the text.
    pub(super) fn encode(&self, text: &str, kind: Kind) -> Result<Encoding> {
        let text = if kind == Kind::Query { truncate_query(text) } else { text };
        self.tokenizer.encode_input(&prefixed(&self.cfg, text, kind))
    }
}

/// Prepends the configured instruction prefix for `kind` to `text`.
///
/// # Arguments
/// * `cfg` - Embedder settings holding `query_prefix` and `doc_prefix` (either may be empty).
/// * `text` - Text to prefix.
/// * `kind` - [`Kind::Query`] uses `query_prefix`; [`Kind::Document`] uses `doc_prefix`.
///
/// # Returns
/// A new string: the prefix followed by `text`.
pub(super) fn prefixed(cfg: &EmbedderConfig, text: &str, kind: Kind) -> String {
    let prefix = match kind {
        Kind::Query => &cfg.query_prefix,
        Kind::Document => &cfg.doc_prefix,
    };
    format!("{prefix}{text}")
}

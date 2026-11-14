//! Shareable tokenizer half of [`super::OrtEmbedder`], used for chunking without the embedder lock.

use tokenizers::{Encoding, TruncationDirection};

use crate::error::Result;
use crate::ports::embedder::Tokenizer;

use super::err;

/// HuggingFace tokenizer plus the token budgets of one model input.
pub struct OrtTokenizer {
    /// Tokenizer with truncation and padding disabled.
    pub inner: tokenizers::Tokenizer,
    /// Instruction prefix prepended to every document.
    doc_prefix: String,
    /// Tokens allowed for prefix plus text: model `max_tokens` minus special tokens. The one
    /// limit both [`OrtTokenizer::encode_input`] and [`Tokenizer::document_overflow`] apply.
    input_limit: usize,
    /// Initial chunk window: `input_limit` minus the tokens of the document prefix alone.
    content_limit: usize,
}

impl OrtTokenizer {
    /// Disables truncation and padding on `inner` and derives the token budgets.
    ///
    /// # Arguments
    /// * `inner` - Tokenizer loaded from `tokenizer.json`.
    /// * `max_tokens` - Largest model input in tokens, special tokens included.
    /// * `doc_prefix` - Instruction prefix prepended to every document before encoding.
    ///
    /// # Returns
    /// The tokenizer with its input limit and initial chunk window.
    ///
    /// # Errors
    /// * [`crate::error::Error::Embedder`] - The tokenizer rejects the settings or fails to encode
    ///   the empty input or `doc_prefix` (the message is the tokenizer's); or `max_tokens` leaves
    ///   no room for document text.
    pub fn new(mut inner: tokenizers::Tokenizer, max_tokens: usize, doc_prefix: &str) -> Result<Self> {
        inner.with_truncation(None).map_err(err)?;
        inner.with_padding(None);
        let specials = inner.encode("", true).map_err(err)?.len();
        let prefix = inner.encode(doc_prefix, false).map_err(err)?.len();
        let input_limit = max_tokens.saturating_sub(specials);
        let content_limit = input_limit.saturating_sub(prefix);
        if content_limit == 0 {
            return Err(err("max_tokens leaves no room for content"));
        }
        Ok(Self { inner, doc_prefix: doc_prefix.to_owned(), input_limit, content_limit })
    }

    /// Tokenises already-prefixed `text`, keeps at most the input limit, and adds special tokens.
    ///
    /// # Arguments
    /// * `text` - Instruction prefix followed by the query or document text.
    ///
    /// # Returns
    /// An encoding of at most `max_tokens` tokens, ready to feed the model.
    ///
    /// # Errors
    /// * [`crate::error::Error::Embedder`] - The tokenizer fails to encode or post-process `text`.
    pub fn encode_input(&self, text: &str) -> Result<Encoding> {
        let mut enc = self.inner.encode(text, false).map_err(err)?;
        enc.truncate(self.input_limit, 0, TruncationDirection::Right);
        self.inner.post_process(enc, None, true).map_err(err)
    }
}

impl Tokenizer for OrtTokenizer {
    /// Tokenises `text` without special tokens and returns each token's byte span.
    ///
    /// # Arguments
    /// * `text` - Text to tokenise.
    ///
    /// # Returns
    /// `(start, end)` byte offsets into `text`, one per token, in order.
    ///
    /// # Errors
    /// * [`crate::error::Error::Embedder`] - The tokenizer fails to encode the text.
    fn token_spans(&self, text: &str) -> Result<Vec<(usize, usize)>> {
        Ok(self.inner.encode(text, false).map_err(err)?.get_offsets().to_vec())
    }

    /// Returns the initial chunk window of one document input.
    ///
    /// # Returns
    /// The input limit minus document-prefix tokens.
    fn max_tokens(&self) -> usize {
        self.content_limit
    }

    /// Encodes the document prefix plus `text` as [`OrtTokenizer::encode_input`] does and counts
    /// the tokens past the input limit.
    ///
    /// # Arguments
    /// * `text` - Candidate chunk text.
    ///
    /// # Returns
    /// How many tokens [`OrtTokenizer::encode_input`] would cut; 0 when the chunk fits whole.
    ///
    /// # Errors
    /// * [`crate::error::Error::Embedder`] - The tokenizer fails to encode the text.
    fn document_overflow(&self, text: &str) -> Result<usize> {
        let enc = self.inner.encode(format!("{}{text}", self.doc_prefix), false).map_err(err)?;
        Ok(enc.len().saturating_sub(self.input_limit))
    }
}

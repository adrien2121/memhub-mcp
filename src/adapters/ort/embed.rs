//! [`Embedder`] port implementation for [`super::OrtEmbedder`].

use std::sync::Arc;

use ort::value::Tensor;

use super::{OrtEmbedder, err, pool};
use crate::error::Result;
use crate::ports::embedder::{self, Embedder, Kind};

impl Embedder for OrtEmbedder {
    /// Encodes `text`, runs one onnxruntime inference on it, and pools the output into one vector.
    ///
    /// Feeds `input_ids`, plus an all-ones `attention_mask` and all-zeros `token_type_ids` when
    /// the model declares those inputs. The vector is not normalised; the app does that.
    ///
    /// # Arguments
    /// * `text` - Query or document text; too-long input is truncated, not rejected.
    /// * `kind` - Selects the query or document instruction prefix.
    ///
    /// # Returns
    /// A vector of exactly `dim` floats.
    ///
    /// # Errors
    /// * [`crate::error::Error::Embedder`] - Tokenising, tensor building or inference fails, or the
    ///   output has an unexpected shape or dimension.
    fn embed(&mut self, text: &str, kind: Kind) -> Result<Vec<f32>> {
        let enc = self.encode(text, kind)?;
        let len = enc.len();
        let tensor = |v: Vec<i64>| Tensor::from_array(([1, len], v)).map_err(err);
        let mut inputs = vec![("input_ids", tensor(enc.get_ids().iter().map(|&i| i64::from(i)).collect())?)];
        if self.use_mask {
            inputs.push(("attention_mask", tensor(vec![1; len])?));
        }
        if self.use_type_ids {
            inputs.push(("token_type_ids", tensor(vec![0; len])?));
        }
        let outputs = self.session.run(inputs).map_err(err)?;
        pool::pool(&self.cfg, &outputs[0])
    }

    /// Returns the configured vector dimension (checked against the model at load time).
    ///
    /// # Returns
    /// Number of floats in every vector from [`Embedder::embed`].
    fn dim(&self) -> usize {
        self.cfg.dim
    }

    /// Renders the [`super::ModelIdentity`] as the text stored in the database `meta` table.
    ///
    /// # Returns
    /// One line listing model id, file hashes, pooling, prefixes, token limit and dimension;
    /// a change means stored vectors are stale.
    fn identity(&self) -> String {
        self.identity.to_string()
    }

    /// Shares the tokenizer so callers can chunk text without holding the embedder lock.
    ///
    /// # Returns
    /// A new reference-counted handle to the same tokenizer.
    fn tokenizer(&self) -> Arc<dyn embedder::Tokenizer> {
        Arc::clone(&self.tokenizer) as Arc<dyn embedder::Tokenizer>
    }
}

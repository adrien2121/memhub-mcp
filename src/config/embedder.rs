//! Embedding model settings: any ONNX sentence-embedding model is selected by config alone.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// onnxruntime shared library file name on this OS.
const ORT_LIB: &str = if cfg!(target_os = "macos") {
    "libonnxruntime.dylib"
} else if cfg!(windows) {
    "onnxruntime.dll"
} else {
    "libonnxruntime.so"
};

/// How token vectors become one text vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pooling {
    /// First token's vector.
    Cls,
    /// Mean of all token vectors.
    Mean,
}

/// Model files, pooling, prefixes and size limits; keys are flat in `config.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedderConfig {
    /// Directory holding `model.onnx` and `tokenizer.json`.
    pub model_dir: PathBuf,
    /// Model repo and revision, recorded in `meta`.
    pub model_id: String,
    /// onnxruntime dynamic library.
    pub ort_dylib: PathBuf,
    /// Pooling of the token vectors.
    pub pooling: Pooling,
    /// Text prepended to queries.
    pub query_prefix: String,
    /// Text prepended to documents.
    pub doc_prefix: String,
    /// Largest model input, special tokens and prefix included.
    pub max_tokens: usize,
    /// Expected vector dimension; checked against the model output.
    pub dim: usize,
}

impl EmbedderConfig {
    /// Builds the default embedder: bge-small-en-v1.5 (int8 ONNX, CLS pooling, 384 dims, 512 tokens) with files under `data`.
    ///
    /// # Arguments
    /// * `data` - Data directory holding `models/` and the onnxruntime library under `lib/`.
    ///
    /// # Returns
    /// The default settings; no files are checked.
    pub fn defaults(data: &Path) -> Self {
        Self {
            model_dir: data.join("models/bge-small-en-v1.5"),
            model_id: "Xenova/bge-small-en-v1.5@ea104dacec62c0de699686887e3f920caeb4f3e3".into(),
            ort_dylib: data.join("lib").join(ORT_LIB),
            pooling: Pooling::Cls,
            query_prefix: "Represent this sentence for searching relevant passages: ".into(),
            doc_prefix: String::new(),
            max_tokens: 512,
            dim: 384,
        }
    }
}

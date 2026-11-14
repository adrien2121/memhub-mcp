//! Regressions for the document-prefix handling of [`super::OrtEmbedder::load`] and the token
//! limit shared by the chunker and the encoder.

use super::encode::prefixed;
use super::*;
use crate::ports::embedder::Kind;

/// Proves `load` fails, carrying the tokenizer's own message, when the tokenizer cannot encode
/// the document prefix, instead of treating the prefix as zero tokens.
///
/// Uses a word-level tokenizer with no unknown token, which errors on any out-of-vocabulary
/// word, and a placeholder `model.onnx`; returns early when the onnxruntime dylib is missing.
#[test]
fn load_fails_when_doc_prefix_cannot_be_encoded() {
    let Ok(mut config) = Config::load() else { return };
    if !config.embedder.ort_dylib.exists() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("memhub-mcp_prefix_{}", std::process::id()));
    let json = r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],
        "normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":null,
        "decoder":null,"model":{"type":"WordLevel","vocab":{"known":0},"unk_token":"[UNK]"}}"#;
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join("tokenizer.json"), json))
        .and_then(|()| std::fs::write(dir.join("model.onnx"), b"not a model"));
    assert!(written.is_ok(), "{written:?}");
    let Ok(tok) = Tokenizer::from_file(dir.join("tokenizer.json")) else { panic!("fixture tokenizer") };
    let Err(cause) = tok.encode("unknown", false) else { panic!("prefix must not encode") };
    config.embedder.model_dir.clone_from(&dir);
    config.embedder.doc_prefix = "unknown".into();
    let result = OrtEmbedder::load(&config).map(|_| ());
    let _ = std::fs::remove_dir_all(&dir);
    let Err(Error::Embedder(msg)) = result else { panic!("expected Embedder error, got {result:?}") };
    assert!(msg.contains(&cause.to_string()), "{msg:?} lacks {cause:?}");
}

/// Asserts every chunk `chunk::split` emits, once the document prefix is prepended, goes through
/// `OrtEmbedder::encode` within the model limit and without truncation.
///
/// Called from the single model-backed test, because onnxruntime allows one environment per
/// process. Swaps in each document prefix (with and without a trailing separator) by rebuilding
/// the tokenizer as `load` does, then restores the original prefix and tokenizer.
///
/// # Arguments
/// * `e` - The loaded embedder.
///
/// # Returns
/// Nothing; `e` is left with its original prefix and tokenizer.
///
/// # Errors
/// None; failures panic.
///
/// # Panics
/// When a chunk is truncated or exceeds `max_tokens`, or when tokenising fails.
pub(super) fn assert_prefixed_chunks_fit(e: &mut OrtEmbedder) {
    let (saved_prefix, saved_tok) = (e.cfg.doc_prefix.clone(), Arc::clone(&e.tokenizer));
    let words: Vec<String> = (0..4000).map(|i| format!("w{}ord{}", i % 7, i % 97)).collect();
    let body = words.join(" ");
    for prefix in ["passage: ", "search_document:", "passage"] {
        let inner = saved_tok.inner.clone();
        let Ok(tok) = OrtTokenizer::new(inner, e.cfg.max_tokens, prefix) else { panic!("{prefix:?}") };
        e.tokenizer = Arc::new(tok);
        e.cfg.doc_prefix = prefix.into();
        let Ok(chunks) = crate::app::chunk::split(&body, e.tokenizer.as_ref()) else { panic!("split") };
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            let Ok(enc) = e.encode(&chunk.text, Kind::Document) else { panic!("encode") };
            let input = prefixed(&e.cfg, &chunk.text, Kind::Document);
            let full = e.tokenizer.inner.encode(input, true).map_or(0, |f| f.len());
            assert!(enc.len() <= e.cfg.max_tokens, "{prefix:?}: {} tokens", enc.len());
            assert_eq!(enc.len(), full, "{prefix:?}: chunk {} truncated", chunk.seq);
        }
    }
    (e.cfg.doc_prefix, e.tokenizer) = (saved_prefix, saved_tok);
}

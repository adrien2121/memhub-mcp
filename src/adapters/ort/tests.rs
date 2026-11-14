//! Runs against the configured model; skipped when its files are missing.

use super::encode::prefixed;
use super::*;
use crate::config::Pooling;
use crate::ports::embedder::{Embedder, Kind};

/// Loads the embedder from the user's configuration so model-backed tests can skip cleanly.
///
/// Prints a skip notice to stderr only when `model.onnx` or the onnxruntime dylib is missing.
///
/// # Returns
/// `None` when the config fails to load or those files are missing; otherwise the embedder.
///
/// # Panics
/// When the model files exist but the embedder fails to load, so a broken model fails the test.
fn embedder() -> Option<OrtEmbedder> {
    let config = Config::load().ok()?;
    let dir = &config.embedder.model_dir;
    if !dir.join("model.onnx").exists() || !config.embedder.ort_dylib.exists() {
        eprintln!("skipped: no model in {}", dir.display());
        return None;
    }
    Some(OrtEmbedder::load(&config).unwrap_or_else(|e| panic!("model present but failed to load: {e}")))
}

/// Reads this process's resident memory through `ps`, for memory logging in tests.
///
/// # Returns
/// Resident size as `"<n> MB"`, `"0 MB"` if unparsable, or the `ps` launch error text.
fn rss_mb() -> String {
    let pid = std::process::id().to_string();
    // Absolute on macOS: a bare name searches PATH and costs syspolicyd checks per call.
    let ps = if cfg!(target_os = "macos") { "/bin/ps" } else { "ps" };
    std::process::Command::new(ps)
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1024.0)
        .map_or_else(|e| e.to_string(), |mb| format!("{mb:.0} MB"))
}

/// Proves the default model (bge-small, an ONNX sentence-embedding model) loads with dim 384,
/// 512 tokens and CLS pooling; embeds
/// deterministically with a finite nonzero vector; applies the query prefix only to queries;
/// reports byte-accurate token spans and a 510-token budget; chunks a 1 MiB body; and survives
/// 200 varied embeds including over-long ones. One test because onnxruntime allows one
/// environment per process; returns early when the model files are absent.
#[test]
fn bge_small_default_config() {
    let Some(mut e) = embedder() else { return };
    eprintln!("rss after load: {}", rss_mb());
    let id = e.model_identity();
    assert_eq!((id.dim, id.max_tokens, id.pooling), (384, 512, Pooling::Cls));
    assert_eq!(id.onnx_sha256.len(), 64);

    let text = "SQLite stores the notes and their vectors.";
    let doc = e.embed(text, Kind::Document).unwrap_or_default();
    assert_eq!(doc.len(), e.dim());
    let mut unit = doc.clone();
    assert!(crate::app::embedding::normalize(&mut unit).is_ok(), "pooled vector is finite and nonzero");
    assert_eq!(e.embed(text, Kind::Document).ok(), Some(doc.clone()));
    let query = e.embed(text, Kind::Query).unwrap_or_default();
    assert_ne!(query, doc, "query prefix not applied");
    let prefixed = format!("{}{text}", e.cfg.query_prefix);
    assert_eq!(e.embed(&prefixed, Kind::Document).ok(), Some(query));

    let spans = e.tokenizer().token_spans("hello world").unwrap_or_default();
    assert_eq!(spans, vec![(0, 5), (6, 11)]);
    assert_eq!(e.tokenizer().max_tokens(), 510);

    let big = "Notes about sqlite and vectors.\n\n".repeat(crate::domain::MAX_BODY / 33);
    let before = rss_mb();
    let chunks = crate::app::chunk::split(&big, e.tokenizer().as_ref()).unwrap_or_default();
    eprintln!("rss before/after chunking a 1 MiB body: {before} / {}", rss_mb());
    assert!(chunks.len() > 100);

    let words: Vec<String> = (0..600).map(|i| format!("word{}", i % 97)).collect();
    for i in 0..200 {
        let n = if i % 20 == 19 { 600 } else { 3 + (i * 37) % 250 };
        assert!(e.embed(&words[..n].join(" "), Kind::Document).is_ok());
    }
    eprintln!("rss after 200 embeds: {}", rss_mb());
    super::limit_tests::assert_prefixed_chunks_fit(&mut e);
}

/// Proves the default bge prefix is prepended to queries and not to documents, by calling
/// `prefixed` on the default configuration (no model needed).
#[test]
fn bge_prefix_applies_to_queries_only() {
    let cfg = EmbedderConfig::defaults(std::path::Path::new("unused"));
    let bge = "Represent this sentence for searching relevant passages: ";
    assert_eq!(prefixed(&cfg, "rust", Kind::Query), format!("{bge}rust"));
    assert_eq!(prefixed(&cfg, "rust", Kind::Document), "rust");
}

//! The real model finds a note by meaning when the query shares no word with it.
//! Skipped when the configured model files are missing.

mod common;

use std::sync::{Arc, Mutex};

use memhub_mcp::adapters::ort::OrtEmbedder;
use memhub_mcp::app::embed_worker;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::config::Config;
use memhub_mcp::domain::{NewNote, SearchFilter, SearchRequest, SearchResults};
use memhub_mcp::error::Result;
use memhub_mcp::ports::embedder::SharedEmbedder;
use memhub_mcp::ports::store::NoteStore;

/// Loads the configured ONNX embedder when both `model.onnx` and the ORT dylib exist.
///
/// Reads `Config::load()`. If either file is missing it prints a skip message to stderr and returns `Ok(None)`. Otherwise it loads `OrtEmbedder` into a shared handle.
///
/// # Returns
/// `Some` shared embedder, or `None` when the model is not installed.
///
/// # Errors
/// * config loading or ONNX model loading fails.
fn real_embedder() -> Result<Option<SharedEmbedder>> {
    let config = Config::load()?;
    let dir = &config.embedder.model_dir;
    if !dir.join("model.onnx").exists() || !config.embedder.ort_dylib.exists() {
        eprintln!("skipped: no model in {}", dir.display());
        return Ok(None);
    }
    Ok(Some(Arc::new(Mutex::new(Box::new(OrtEmbedder::load(&config)?)))))
}

/// Runs `query` through `svc` with limit 10 and no filter.
///
/// # Arguments
/// * `svc` - search service to query.
/// * `query` - search text.
///
/// # Returns
/// The search results.
///
/// # Errors
/// * propagates search failures.
fn search(svc: &SearchService, query: &str) -> Result<SearchResults> {
    svc.search(&SearchRequest { query: query.into(), limit: 10, filter: SearchFilter::default() })
}

/// Proves the real model finds a note by meaning when the query shares no word with it. Skips, passing trivially, when no model is installed. Three notes are written and embedded. Keyword search for `vehicle wheels` must return no hits, and hybrid search must rank the `car` note first without being keyword-only.
#[test]
fn paraphrase_without_shared_words_ranks_target_first() -> Result<()> {
    let Some(e) = real_embedder()? else { return Ok(()) };
    let store: Arc<dyn NoteStore> = common::store("semantic_recall")?;
    embed_worker::adopt_if_fresh(store.as_ref(), &e)?;
    let notes = NoteService::new(Arc::clone(&store), Some(Arc::clone(&e)), None);
    let mut target = None;
    for (title, body) in [
        ("garden", "Water the tomato plants every morning before work."),
        ("car", "The automobile needs new tyres before winter."),
        ("taxes", "File the annual income declaration by April."),
    ] {
        let n = notes.write(NewNote { title: title.into(), body: body.into(), tags: vec![] }, false)?;
        if title == "car" {
            target = Some(n.id);
        }
    }
    embed_worker::drain(store.as_ref(), &e, &mut |_| {})?;
    let query = "vehicle wheels";
    let keyword = SearchService::new(Arc::clone(&store), None);
    assert!(search(&keyword, query)?.hits.is_empty(), "query shares a word with a note");
    let hybrid = search(&SearchService::new(Arc::clone(&store), Some(e)), query)?;
    assert!(!hybrid.keyword_only);
    assert_eq!(hybrid.hits.first().map(|h| h.note_id), target);
    eprintln!("ran with real model");
    Ok(())
}

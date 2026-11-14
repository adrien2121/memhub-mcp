//! Hybrid search: FTS5 keyword scores and exact vector scores, fused by convex combination.

use std::collections::HashMap;
use std::sync::Arc;

use crate::app::{embed_worker, embedding, fuse::fuse, nearest::nearest, query, reembed};
use crate::domain::{ChunkId, NoteId, SearchHit, SearchRequest, SearchResults, truncate_query};
use crate::error::Result;
use crate::ports::embedder::{Kind, SharedEmbedder};
use crate::ports::store::NoteStore;

/// Most hits one search can return.
const MAX_LIMIT: usize = 100;
/// Candidates taken from each list before fusion.
const DEPTH: usize = 100;
/// Weight of the vector list in the fusion; the keyword list gets `1 - ALPHA`.
const ALPHA: f64 = 0.5;

/// Runs searches against a [`NoteStore`] and an optional embedder.
pub struct SearchService {
    store: Arc<dyn NoteStore>,
    embedder: Option<SharedEmbedder>,
}

impl SearchService {
    /// Creates the service.
    ///
    /// # Arguments
    /// * `store` - Note store to search.
    /// * `embedder` - Configured embedder; `None` makes every search keyword-only.
    ///
    /// # Returns
    /// The service.
    pub fn new(store: Arc<dyn NoteStore>, embedder: Option<SharedEmbedder>) -> Self {
        Self { store, embedder }
    }

    /// Searches notes matching `req.filter`, best first: FTS5 keyword hits and vector hits, each
    /// up to 100 deep, fused with equal weight. Keyword-only when the embedder is missing, fails
    /// or panics, or the model identity mismatches. Chunks still pending embedding are found by
    /// keyword only. Read-only; locks the embedder once for the query vector.
    ///
    /// # Arguments
    /// * `req` - Query text (cut at the byte cap), hit limit (clamped to 1..=100) and filters.
    ///
    /// # Returns
    /// Hits with snippets (keyword snippet, else the best chunk's text); hybrid hits carry their
    /// fused position as `rank`, keyword-only hits their BM25 value. Also reports whether the
    /// search was keyword-only, pending and failed embedding counts, words dropped past the term
    /// cap, and on keyword-only searches any embedder change. No hits when the query has no word.
    ///
    /// # Errors
    /// * [`Error::Storage`](crate::error::Error::Storage) - Any store call fails.
    pub fn search(&self, req: &SearchRequest) -> Result<SearchResults> {
        let limit = req.limit.clamp(1, MAX_LIMIT);
        let text = truncate_query(&req.query);
        let pending_embeddings = self.store.pending_count()?;
        let failed_embeddings = self.store.failed_count()?;
        let stored = self.store.model_identity()?;
        let fts = query::build(text);
        let (configured, vector) = self.embed_query(stored.as_deref(), fts.as_ref().map(|_| text));
        let embedder_changed = match (&stored, &configured) {
            (Some(s), Some(c)) => reembed::change_between(self.store.as_ref(), s, c)?,
            _ => None,
        };
        let keyword_only = |hits, dropped_terms| SearchResults {
            hits,
            keyword_only: true,
            pending_embeddings,
            failed_embeddings,
            dropped_terms,
            embedder_changed: embedder_changed.clone(),
        };
        let Some(fts) = fts else { return Ok(keyword_only(Vec::new(), 0)) };
        let dropped_terms = fts.dropped;
        let keyword = self.store.fts_search(&fts.query, &req.filter, DEPTH)?;
        let Some(vector) = vector else {
            let hits = keyword
                .into_iter()
                .take(limit)
                .map(|(note_id, rank)| {
                    Ok(SearchHit { note_id, rank, snippet: self.store.fts_snippet(&fts.query, note_id)? })
                })
                .collect::<Result<_>>()?;
            return Ok(keyword_only(hits, dropped_terms));
        };
        let vector = nearest(self.store.as_ref(), &req.filter, &vector, DEPTH)?;
        let kw: Vec<(NoteId, f64)> = keyword.iter().map(|&(n, rank)| (n, -rank)).collect();
        let vec: Vec<(NoteId, f64)> = vector.iter().map(|v| (v.0, v.1)).collect();
        let chunks: HashMap<NoteId, ChunkId> = vector.into_iter().map(|(n, _, c)| (n, c)).collect();
        let mut hits = Vec::with_capacity(limit);
        for (pos, note_id) in fuse(&kw, &vec, ALPHA).into_iter().take(limit).enumerate() {
            let snippet = if keyword.iter().any(|&(n, _)| n == note_id) {
                self.store.fts_snippet(&fts.query, note_id)?
            } else if let Some(c) = chunks.get(&note_id) {
                self.store.chunk_text(*c)?.unwrap_or_default()
            } else {
                String::new()
            };
            hits.push(SearchHit { note_id, rank: (pos + 1) as f64, snippet });
        }
        Ok(SearchResults { keyword_only: false, embedder_changed: None, ..keyword_only(hits, dropped_terms) })
    }

    /// Reads the configured embedder identity and, if it matches `stored`, embeds the query,
    /// under one embedder lock; failures and panics are logged as warnings, not returned.
    ///
    /// # Arguments
    /// * `stored` - Identity recorded with the stored embeddings, if any.
    /// * `query` - Query text to embed; `None` skips embedding.
    ///
    /// # Returns
    /// (configured identity, query vector). The vector is `None` when `query` is `None` or the
    /// identities differ; both are `None` when there is no embedder or it fails.
    fn embed_query(&self, stored: Option<&str>, query: Option<&str>) -> (Option<String>, Option<Vec<f32>>) {
        let Some(embedder) = &self.embedder else { return (None, None) };
        let embedded = embed_worker::guarded(embedder, |e| {
            let identity = e.identity();
            let matches = query.filter(|_| stored == Some(identity.as_str()));
            let vector = matches.map(|q| embedding::embed(e, q, Kind::Query)).transpose()?;
            Ok((identity, vector))
        });
        embedded
            .inspect_err(|err| tracing::warn!("vector search skipped: {err}"))
            .map_or((None, None), |(identity, vector)| (Some(identity), vector))
    }
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;

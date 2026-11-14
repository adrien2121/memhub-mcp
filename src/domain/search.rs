//! Search request and result types.

use serde::{Deserialize, Serialize};

use super::{SearchHit, Tag};

/// Filters applied inside the store's search query.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchFilter {
    /// Only notes carrying this tag.
    pub tag: Option<Tag>,
    /// Only notes updated at or after this Unix time.
    pub updated_after: Option<i64>,
    /// Only notes updated before this Unix time.
    pub updated_before: Option<i64>,
}

/// A search request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    /// Free-text query.
    pub query: String,
    /// Maximum hits, clamped to 1..=100.
    pub limit: usize,
    /// Tag and date filters.
    pub filter: SearchFilter,
}

/// Search results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    /// Hits, best first.
    pub hits: Vec<SearchHit>,
    /// True when only keyword search ran.
    pub keyword_only: bool,
    /// Chunks not yet embedded; their notes are found by keyword only until this reaches 0.
    pub pending_embeddings: usize,
    /// Chunks whose embedding failed too often to retry; found by keyword only until a `reembed`.
    #[serde(default)]
    pub failed_embeddings: usize,
    /// Query words past the 32-term cap that keyword search ignored; 0 when none.
    #[serde(default)]
    pub dropped_terms: usize,
    /// Set while the configured embedder differs from the one the stored vectors came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedder_changed: Option<EmbedderChange>,
}

/// The configured embedder differs from the stored vectors; vector search is off until `reembed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedderChange {
    /// Model the stored vectors were made with.
    pub stored: String,
    /// Model in the current config.
    pub configured: String,
    /// Chunks a re-embed would process.
    pub chunks: usize,
    /// Rough re-embed duration in seconds.
    pub est_seconds: f64,
}

/// Outcome of the `reembed` tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReembedStatus {
    /// True when this call reset the vectors and queued every chunk.
    pub started: bool,
    /// The pending model change, if any (before this call when `started`).
    pub embedder_changed: Option<EmbedderChange>,
    /// Chunks awaiting an embedding.
    pub pending_embeddings: usize,
    /// Chunks whose embedding failed too often to retry.
    #[serde(default)]
    pub failed_embeddings: usize,
}

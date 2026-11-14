//! MCP `search_notes` tool.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::schemars::JsonSchema;
use rmcp::{tool, tool_router};
use serde::Deserialize;

use super::server::MemoryServer;
use crate::domain::{SearchFilter, SearchRequest, Tag};

#[derive(Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct SearchArgs {
    /// Free-text query; only the first 2 KiB (2,048 bytes) are used.
    query: String,
    /// Most results returned (default 10).
    limit: Option<usize>,
    /// Only notes with this tag.
    tag: Option<String>,
    /// Only notes updated at or after this Unix time (seconds).
    updated_after: Option<i64>,
    /// Only notes updated before this Unix time (seconds).
    updated_before: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct ReembedArgs {
    /// True only after the user agreed to re-embed; false just reports status.
    confirm: bool,
}

#[tool_router(router = search_router, vis = "pub(crate)")]
impl MemoryServer {
    /// `search_notes` tool: runs a hybrid keyword/vector search with optional filters.
    ///
    /// # Arguments
    /// * `a` - Query text (only the first 2,048 bytes are used), result limit (default 10), and
    ///   optional tag and update-time bounds (Unix seconds).
    ///
    /// # Returns
    /// The search response as JSON (hits plus `keyword_only`, `pending_embeddings`,
    /// `failed_embeddings`, `embedder_changed` and `dropped_terms`), or an `isError` result if the search fails.
    #[tool(
        description = "Search notes by text. Results say whether the search was keyword-only and how many chunks still await embedding (`pending_embeddings`; those notes match by keyword only). `embedder_changed` means the configured embedding model differs from the stored vectors: tell the user and offer the `reembed` tool. `dropped_terms` counts query words past the 32-word keyword cap; shorten the query when it is nonzero."
    )]
    async fn search_notes(&self, Parameters(a): Parameters<SearchArgs>) -> CallToolResult {
        let search = self.search.clone();
        let filter =
            SearchFilter { tag: a.tag.map(Tag), updated_after: a.updated_after, updated_before: a.updated_before };
        let request = SearchRequest { query: a.query, limit: a.limit.unwrap_or(10), filter };
        self.call(move || search.search(&request)).await
    }

    /// `reembed` tool: reports re-embedding status, or with `confirm` starts re-embedding every
    /// note with the configured model and returns immediately.
    ///
    /// # Arguments
    /// * `a` - `confirm`: true only after the user agreed; false only reports status.
    ///
    /// # Returns
    /// The re-embed status (including the estimated duration) as JSON, or an `isError` result
    /// if the service call fails.
    #[tool(
        description = "Re-embed every note with the configured embedding model. Use when search reports `embedder_changed`: the configured model differs from the stored vectors, so search is keyword-only. Re-embedding replaces all stored vectors and takes about `est_seconds`; ASK THE USER before calling with confirm=true. confirm=false (or no change) only returns status. confirm=true returns at once; searches then report `pending_embeddings` counting down to 0."
    )]
    async fn reembed(&self, Parameters(a): Parameters<ReembedArgs>) -> CallToolResult {
        let reembed = self.reembed.clone();
        self.call(move || reembed.reembed(a.confirm)).await
    }
}

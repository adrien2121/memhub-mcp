//! MCP server setup and tool routing.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ServerHandler, tool_handler};
use serde::Serialize;
use tokio::sync::Semaphore;

use crate::adapters::sqlite::READ_POOL;
use crate::app::notes::NoteService;
use crate::app::reembed::ReembedService;
use crate::app::search::SearchService;
use crate::domain::{Note, NoteId};
use crate::error::Result;

/// What `write_note` and `edit_note` return: enough to chain the next edit, without echoing the body.
#[derive(Serialize)]
pub(crate) struct Receipt {
    id: NoteId,
    version: i64,
    updated_at: i64,
}

impl From<Note> for Receipt {
    /// Builds a receipt from a stored note, keeping only its id, version and update time.
    ///
    /// # Arguments
    /// * `n` - The note just written or edited; its body and tags are dropped.
    ///
    /// # Returns
    /// A [`Receipt`] carrying `n.id`, `n.version` and `n.updated_at` (Unix seconds).
    fn from(n: Note) -> Self {
        Self { id: n.id, version: n.version, updated_at: n.updated_at }
    }
}

/// One MCP server shared by every session; cloning is cheap.
#[derive(Clone)]
pub struct MemoryServer {
    pub(crate) notes: Arc<NoteService>,
    pub(crate) search: Arc<SearchService>,
    pub(crate) reembed: Arc<ReembedService>,
    /// Bounds blocking service calls across all sessions to the read pool size.
    workers: Arc<Semaphore>,
    router: ToolRouter<Self>,
}

impl MemoryServer {
    /// Builds the server, combining the note and search tool routers into one router.
    ///
    /// The worker semaphore is sized to [`READ_POOL`] so blocking service calls across all
    /// sessions never outnumber the SQLite read connections.
    ///
    /// # Arguments
    /// * `notes` - Shared note service backing write/read/edit/delete/list tools.
    /// * `search` - Shared search service backing `search_notes`.
    /// * `reembed` - Shared re-embedding service backing `reembed`.
    ///
    /// # Returns
    /// A cloneable server whose clones share the services and the worker semaphore.
    pub fn new(notes: Arc<NoteService>, search: Arc<SearchService>, reembed: Arc<ReembedService>) -> Self {
        let workers = Arc::new(Semaphore::new(READ_POOL));
        Self { notes, search, reembed, workers, router: Self::notes_router() + Self::search_router() }
    }

    /// Waits for a worker slot, runs a service call on a blocking thread, and turns its outcome
    /// into a tool result.
    ///
    /// The slot is released as soon as `f` finishes (before encoding). Panics in `f` are caught.
    ///
    /// # Arguments
    /// * `f` - The blocking service call to run; it must not borrow from the caller.
    ///
    /// # Returns
    /// A success result whose single text block is `f`'s value as JSON, or an `isError` result
    /// holding the service error's message, `internal error: the tool panicked`,
    /// `internal error: worker pool closed`, or a JSON encoding failure message.
    pub(crate) async fn call<T, F>(&self, f: F) -> CallToolResult
    where
        F: FnOnce() -> Result<T> + Send + 'static,
        T: Serialize + Send + 'static,
    {
        let Ok(slot) = Arc::clone(&self.workers).acquire_owned().await else {
            return failure("internal error: worker pool closed".into());
        };
        let joined = tokio::task::spawn_blocking(move || {
            let out = std::panic::catch_unwind(AssertUnwindSafe(f));
            drop(slot);
            out
        })
        .await;
        match joined {
            Ok(Ok(Ok(value))) => match rmcp::serde_json::to_string(&value) {
                Ok(json) => CallToolResult::success(vec![ContentBlock::text(json)]),
                Err(e) => failure(format!("internal error: cannot encode result: {e}")),
            },
            Ok(Ok(Err(e))) => failure(e.to_string()),
            Ok(Err(_)) | Err(_) => failure("internal error: the tool panicked".into()),
        }
    }
}

#[tool_handler(
    router = self.router,
    name = "memhub-mcp",
    instructions = "memhub-mcp is your long-term memory: notes that persist across sessions and projects. \
Before starting a task, search notes for related past work. After solving a problem, making a decision or \
learning a fact, save a note. Edit an existing note instead of writing a duplicate."
)]
impl ServerHandler for MemoryServer {}

/// Builds an `isError` tool result whose single text block is `message`.
///
/// # Arguments
/// * `message` - Human-readable error shown to the MCP client.
///
/// # Returns
/// A [`CallToolResult`] flagged as an error.
pub(crate) fn failure(message: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

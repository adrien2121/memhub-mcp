//! Per-session transport: bounds a session's line length and its unanswered requests.
//!
//! Once [`MAX_OUTSTANDING`] requests are running or await a reply that has been written out,
//! the session stops reading its input, so a client that floods requests without reading
//! replies (or issues and cancels them) stalls on its own socket instead of growing the
//! daemon's queues.

mod transport;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rmcp::RoleServer;
use rmcp::model::RequestId;
use rmcp::transport::async_rw::AsyncRwTransport;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::domain::MAX_BODY;

/// Requests one session may have in flight before its input is no longer read.
pub const MAX_OUTSTANDING: usize = 32;

/// Room for everything in a request line besides the body: the JSON-RPC envelope, tool name,
/// title (at most 200 chars, 1.2 KB escaped), tags and edit-op fields. 64 KiB leaves slack for a
/// body slightly over [`MAX_BODY`] to still reach the size check and get its error reply.
const ENVELOPE: usize = 64 * 1024;

/// Longest accepted input line; longer lines are dropped unanswered (they carry no readable id).
/// Covers a [`MAX_BODY`] body at JSON's worst-case escaping (6 bytes per `\u00XX` char), so every
/// in-limit request gets a reply.
pub const MAX_LINE: usize = MAX_BODY * 6 + ENVELOPE;

/// One share of a request's slot; the permit frees when every share is dropped.
#[derive(Clone)]
pub(crate) struct Slot(#[allow(dead_code)] Arc<OwnedSemaphorePermit>);

/// In-flight request ids mapped to the slot share held until the reply is written or cancelled.
type Held = Arc<Mutex<HashMap<RequestId, Slot>>>;

/// An MCP server transport over one byte stream, with the bounds above.
pub struct Session<R: AsyncRead, W: AsyncWrite> {
    inner: AsyncRwTransport<RoleServer, R, W>,
    slots: Arc<Semaphore>,
    /// A slot taken by a `receive` that was cancelled before a message arrived.
    ready: Option<OwnedSemaphorePermit>,
    held: Held,
}

impl<R, W> Session<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    /// Wraps the read and write halves of a client connection in a bounded MCP transport.
    ///
    /// Lines longer than [`MAX_LINE`] bytes are dropped, and at most [`MAX_OUTSTANDING`]
    /// requests may await a reply before input stops being read.
    ///
    /// # Arguments
    /// * `read` - The client's input half (e.g. a Unix socket reader); owned by the session.
    /// * `write` - The client's output half; owned by the session.
    ///
    /// # Returns
    /// A session with no requests in flight.
    pub fn new(read: R, write: W) -> Self {
        Self {
            inner: AsyncRwTransport::new_server(read, write).with_max_line_length(MAX_LINE),
            slots: Arc::new(Semaphore::new(MAX_OUTSTANDING)),
            ready: None,
            held: Arc::default(),
        }
    }
}

//! End to end over the real server and session transport: duplicate in-flight ids, write/edit
//! receipts, list limits and body size limit.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use memhub_mcp::adapters::sqlite::SqliteStore;
use memhub_mcp::app::notes::NoteService;
use memhub_mcp::app::reembed::ReembedService;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::domain::MAX_BODY;
use memhub_mcp::mcp::server::MemoryServer;
use memhub_mcp::mcp::session::Session;
use memhub_mcp::ports::store::NoteStore;
use rmcp::ServiceExt;
use rmcp::serde_json::{Value, from_str, json};
use tokio::io::DuplexStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf};

struct Client {
    w: WriteHalf<DuplexStream>,
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
}

impl Client {
    /// Starts the server over a duplex pipe backed by a fresh SQLite store, then completes the MCP `initialize` handshake.
    ///
    /// Side effects: removes and recreates `memhub-mcp_e2e_<pid>_<n>` under the temp dir (`n` unique per call, so
    /// parallel tests don't delete each other's store), and spawns the server task on the tokio runtime.
    ///
    /// # Returns
    /// A `Client` connected to the server, after the `initialized` notification has been sent.
    ///
    /// # Panics
    /// Panics on any filesystem, store-open, or pipe error.
    async fn start() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("memhub-mcp_e2e_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn NoteStore> =
            Arc::new(SqliteStore::open_with_tokenizer(&dir.join("m.db"), "porter unicode61").unwrap());
        let notes = Arc::new(NoteService::new(Arc::clone(&store), None, None));
        let search = Arc::new(SearchService::new(Arc::clone(&store), None));
        let server = MemoryServer::new(notes, search, Arc::new(ReembedService::new(store, None, None)));
        let (client, srv) = tokio::io::duplex(8 << 20);
        let (sr, sw) = tokio::io::split(srv);
        tokio::spawn(async move {
            if let Ok(running) = server.serve(Session::new(sr, sw)).await {
                let _ = running.waiting().await;
            }
        });
        let (cr, w) = tokio::io::split(client);
        let mut c = Self { w, lines: BufReader::new(cr).lines() };
        c.send(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}})).await;
        c.next().await;
        c.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await;
        c
    }

    /// Serialises `v` as one JSON line and writes it to the server. Does not read a reply.
    ///
    /// # Arguments
    /// * `v` - JSON-RPC message to send.
    ///
    /// # Panics
    /// Panics if the write fails.
    async fn send(&mut self, v: &Value) {
        self.w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
    }

    /// Reads and parses the next JSON line from the server.
    ///
    /// # Returns
    /// The parsed JSON value.
    ///
    /// # Panics
    /// Panics if the stream closes or the line is not valid JSON.
    async fn next(&mut self) -> Value {
        from_str(&self.lines.next_line().await.unwrap().unwrap()).unwrap()
    }

    /// Sends a `tools/call` request for `tool` with `args` under request `id`, then returns the reply.
    ///
    /// # Arguments
    /// * `id` - JSON-RPC request id; the reply must echo it.
    /// * `tool` - MCP tool name.
    /// * `args` - JSON arguments object.
    ///
    /// # Returns
    /// The full JSON-RPC reply.
    ///
    /// # Panics
    /// Panics if the reply id differs from `id` or the stream closes.
    async fn call(&mut self, id: u32, tool: &str, args: Value) -> Value {
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":tool,"arguments":args}}))
            .await;
        let reply = self.next().await;
        assert_eq!(reply["id"], id, "{reply}");
        reply
    }
}

/// Extracts the first text content block from a tool reply.
///
/// # Arguments
/// * `reply` - JSON-RPC reply to inspect.
///
/// # Returns
/// The text, or an empty string if the block is absent.
fn text(reply: &Value) -> &str {
    reply["result"]["content"][0]["text"].as_str().unwrap_or_default()
}

/// Reports whether a tool reply set `isError` to true.
///
/// # Arguments
/// * `reply` - JSON-RPC reply to inspect.
///
/// # Returns
/// `true` only when `result.isError` is exactly `true`.
fn is_error(reply: &Value) -> bool {
    reply["result"]["isError"] == true
}

/// Parses a receipt's text body as JSON and returns its object keys, sorted.
///
/// # Arguments
/// * `reply` - tool reply whose text is a JSON object.
///
/// # Returns
/// The sorted key names.
///
/// # Panics
/// Panics if the text is not JSON or not a JSON object.
fn receipt_keys(reply: &Value) -> Vec<String> {
    let body: Value = from_str(text(reply)).expect("receipt json");
    let mut keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

/// Drives the server end to end over the real session transport: write and edit receipts, list limits, oversized body rejection, and duplicate in-flight request ids.
///
/// Each step asserts the reply shape or error code.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_stack_end_to_end() {
    let mut c = Client::start().await;

    let w = c.call(1, "write_note", json!({"title":"t","body":"hello"})).await;
    assert_eq!(receipt_keys(&w), ["id", "updated_at", "version"], "{w}");
    let e = c.call(2, "edit_note", json!({"id":1,"expected_version":1,"append":" more"})).await;
    assert_eq!(receipt_keys(&e), ["id", "updated_at", "version"], "{e}");

    for i in 0..105 {
        c.call(10 + i, "write_note", json!({"title": format!("n{i}"), "body":"x"})).await;
    }
    let zero = c.call(200, "list_notes", json!({"limit":0})).await;
    assert!(is_error(&zero), "{zero}");
    let big = c.call(201, "list_notes", json!({"limit":1000})).await;
    assert!(!is_error(&big), "{big}");
    let page: Value = from_str(text(&big)).unwrap();
    let items = page.as_object().unwrap().values().find_map(Value::as_array).expect("item array");
    assert_eq!(items.len(), 100, "{big}");

    let huge = "a".repeat(MAX_BODY + 1);
    let over = c.call(202, "write_note", json!({"title":"huge","body":huge})).await;
    assert!(is_error(&over), "oversized body accepted");

    // Duplicate while the first (a large write) is still in flight.
    let slow = "word ".repeat(MAX_BODY / 5 - 1);
    let first = json!({"jsonrpc":"2.0","id":300,"method":"tools/call","params":{"name":"write_note","arguments":{"title":"slow","body":slow}}});
    c.w.write_all(format!("{first}\n{first}\n").as_bytes()).await.unwrap();
    let (a, b) = (c.next().await, c.next().await);
    let dup = [&a, &b].into_iter().find(|r| r.get("error").is_some()).expect("no error for duplicate id");
    assert_eq!(dup["error"]["code"], -32600, "{dup}");
    assert_eq!(dup["id"], 300);
    let ok = [&a, &b].into_iter().find(|r| r.get("result").is_some()).expect("original request lost");
    assert_eq!(receipt_keys(ok), ["id", "updated_at", "version"]);
}

/// Regression: a `MAX_BODY` body that JSON-escapes to six bytes per char (`\u0001`) must still
/// get a reply over the real session transport, and a body just over `MAX_BODY` the size error.
#[tokio::test(flavor = "multi_thread")]
async fn worst_case_escaped_body_gets_reply() {
    let mut c = Client::start().await;
    let fits = "\u{1}".repeat(MAX_BODY);
    let ok = tokio::time::timeout(Duration::from_secs(60), c.call(1, "write_note", json!({"title":"esc","body":fits})))
        .await
        .expect("no reply to a MAX_BODY request");
    assert!(!is_error(&ok), "{ok}");
    let over = "\u{1}".repeat(MAX_BODY + 1);
    let err =
        tokio::time::timeout(Duration::from_secs(60), c.call(2, "write_note", json!({"title":"esc2","body":over})))
            .await
            .expect("no reply to an oversized request");
    assert!(is_error(&err) && text(&err).contains("over the"), "{err}");
}

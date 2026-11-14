//! A panic inside a tool handler reaches the client as an `isError` result, not a crash.

mod common;

use std::sync::{Arc, Mutex};

use memhub_mcp::app::notes::NoteService;
use memhub_mcp::app::reembed::ReembedService;
use memhub_mcp::app::search::SearchService;
use memhub_mcp::error::{Error, Result};
use memhub_mcp::mcp::server::MemoryServer;
use memhub_mcp::ports::embedder::{Embedder, Kind, SharedEmbedder, Tokenizer};
use memhub_mcp::ports::store::NoteStore;
use rmcp::ServiceExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Panics whenever a tool touches it.
struct PanickingEmbedder;

impl Embedder for PanickingEmbedder {
    /// Always panics: no test should reach this embedder's `embed`.
    ///
    /// # Arguments
    /// * `_` - input text, ignored.
    /// * `_` - embedding kind, ignored.
    ///
    /// # Returns
    /// Never returns.
    ///
    /// # Panics
    /// Always, with `embedder exploded`.
    fn embed(&mut self, _: &str, _: Kind) -> Result<Vec<f32>> {
        panic!("embedder exploded")
    }
    /// Always panics: no test should reach this embedder's `dim`.
    ///
    /// # Returns
    /// Never returns.
    ///
    /// # Panics
    /// Always, with `embedder exploded`.
    fn dim(&self) -> usize {
        panic!("embedder exploded")
    }
    /// Always panics: no test should reach this embedder's `identity`.
    ///
    /// # Returns
    /// Never returns.
    ///
    /// # Panics
    /// Always, with `embedder exploded`.
    fn identity(&self) -> String {
        panic!("embedder exploded")
    }
    /// Always panics: no test should reach this embedder's `tokenizer`.
    ///
    /// # Returns
    /// Never returns.
    ///
    /// # Panics
    /// Always, with `embedder exploded`.
    fn tokenizer(&self) -> Arc<dyn Tokenizer> {
        panic!("embedder exploded")
    }
}

/// Checks that a panic inside the `reembed` tool reaches the client as an `isError` result, and that the server still answers afterwards.
///
/// Setup: an in-process server over a 64 KiB duplex pipe, backed by a `common::store` and the panicking embedder. Sends `initialize`, then `reembed`, then a `read_note` for a missing id.
///
/// # Errors
/// * Propagates store, I/O, and parse errors.
///
/// # Panics
/// Panics if the reply is not `isError` mentioning `panicked`, or if a later reply id is wrong.
#[tokio::test(flavor = "multi_thread")]
async fn panicking_tool_returns_is_error_and_server_survives() -> Result<()> {
    let store: Arc<dyn NoteStore> = common::store("panic_is_error")?;
    store.reset_embeddings("stored-model")?;
    let embedder: SharedEmbedder = Arc::new(Mutex::new(Box::new(PanickingEmbedder)));
    let notes = Arc::new(NoteService::new(Arc::clone(&store), None, None));
    let search = Arc::new(SearchService::new(Arc::clone(&store), None));
    let server = MemoryServer::new(notes, search, Arc::new(ReembedService::new(store, Some(embedder), None)));
    let (client, srv) = tokio::io::duplex(1 << 16);
    let (sr, sw) = tokio::io::split(srv);
    tokio::spawn(async move {
        if let Ok(running) = server.serve((sr, sw)).await {
            let _ = running.waiting().await;
        }
    });
    let (cr, mut cw) = tokio::io::split(client);
    let mut lines = BufReader::new(cr).lines();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let call =
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"reembed","arguments":{"confirm":false}}}"#;
    let after = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_note","arguments":{"id":99}}}"#;
    cw.write_all(format!("{init}\n").as_bytes()).await?;
    assert!(lines.next_line().await?.is_some_and(|l| l.contains("\"id\":1")));
    cw.write_all(format!("{initialized}\n{call}\n").as_bytes()).await?;
    let v = parse(next(&mut lines).await?)?;
    assert_eq!(v["id"], 2);
    assert_eq!(v["result"]["isError"], true, "{v}");
    assert!(v["result"]["content"][0]["text"].as_str().is_some_and(|t| t.contains("panicked")), "{v}");
    // The server still answers after the panic.
    cw.write_all(format!("{after}\n").as_bytes()).await?;
    let v = parse(next(&mut lines).await?)?;
    assert_eq!(v["id"], 3, "{v}");
    Ok(())
}

/// Reads the next reply line from `lines`. A missing reply fails rather than hangs: a 10 s timeout becomes an `InvalidInput` error.
///
/// # Arguments
/// * `lines` - line reader over the server's output.
///
/// # Returns
/// The raw reply line.
///
/// # Errors
/// * Timeout returns `InvalidInput` ("no reply").
/// * A closed stream returns `InvalidInput`.
/// * I/O errors are propagated.
async fn next<R: tokio::io::AsyncBufRead + Unpin>(lines: &mut tokio::io::Lines<R>) -> Result<String> {
    let line = tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line()).await;
    line.map_err(|_| Error::InvalidInput("no reply: the server dropped the call".into()))??
        .ok_or_else(|| Error::InvalidInput("server closed the stream".into()))
}

/// Parses one reply line into a JSON value.
///
/// # Arguments
/// * `line` - raw JSON text.
///
/// # Returns
/// The parsed JSON value.
///
/// # Errors
/// * Invalid JSON returns `InvalidInput` containing the line.
fn parse(line: String) -> Result<rmcp::serde_json::Value> {
    rmcp::serde_json::from_str(&line).map_err(|e| Error::InvalidInput(format!("{e}: {line:?}")))
}

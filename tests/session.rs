//! Session transport: a request reusing an in-flight id is refused and never reaches the server.

use memhub_mcp::mcp::session::Session;
use rmcp::model::{JsonRpcMessage, NumberOrString};
use rmcp::transport::Transport;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Builds a newline-terminated JSON-RPC `tools/call` request for `list_notes` with the given request id.
///
/// # Arguments
/// * `id` - JSON-RPC request id.
///
/// # Returns
/// The serialized request line, ending in `\n`.
fn call(id: u32) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"list_notes","arguments":{{}}}}}}"#)
        + "\n"
}

/// Proves a request that reuses an in-flight id is refused and never reaches the server. Ids 1, 1 and 2 are sent over an in-memory duplex. The session must yield only requests 1 and 2, and the reply line must be a `-32600` error with id 1.
#[tokio::test]
async fn a_request_reusing_an_in_flight_id_is_refused() -> Res<()> {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (server_r, server_w) = tokio::io::split(server);
    let (client_r, mut client_w) = tokio::io::split(client);
    let mut session = Session::new(server_r, server_w);
    client_w.write_all((call(1) + &call(1) + &call(2)).as_bytes()).await?;

    let mut ids = Vec::new();
    for _ in 0..2 {
        match session.receive().await {
            Some(JsonRpcMessage::Request(r)) => ids.push(r.id),
            other => return Err(format!("expected a request, got {other:?}").into()),
        }
    }
    assert_eq!(ids, [NumberOrString::Number(1), NumberOrString::Number(2)], "duplicate reached the server");

    let mut line = String::new();
    BufReader::new(client_r).read_line(&mut line).await?;
    assert!(line.contains("\"id\":1") && line.contains("-32600"), "no error reply for the duplicate: {line}");
    Ok(())
}

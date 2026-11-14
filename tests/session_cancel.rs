//! Session limiter: cancelling a request must not free its slot while its handler still runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use memhub_mcp::mcp::session::{MAX_OUTSTANDING, Session};
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ErrorData};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// A server whose tool calls block on `gate` and ignore cancellation, counting live handlers.
#[derive(Clone)]
struct Blocked {
    gate: Arc<Semaphore>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

impl Default for Blocked {
    fn default() -> Self {
        Self { gate: Arc::new(Semaphore::new(0)), active: Arc::default(), peak: Arc::default() }
    }
}

impl ServerHandler for Blocked {
    async fn call_tool(
        &self,
        _: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        let _pass = self.gate.acquire().await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(CallToolResult::success(vec![]).into())
    }
}

/// One JSON-RPC line.
fn line(body: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0",{body}}}"#) + "\n"
}

/// Polls until `f` holds, failing after two seconds.
async fn until(f: impl Fn() -> bool) -> Res<()> {
    for _ in 0..200 {
        if f() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err("condition not reached".into())
}

/// Proves cancelled requests keep their slot until their handler exits. 32 blocked calls are
/// cancelled and 32 more are sent: no more than 32 handlers may run, and once the first batch is
/// released the second batch runs.
#[tokio::test]
async fn cancelling_requests_does_not_exceed_the_handler_cap() -> Res<()> {
    let (client, server) = tokio::io::duplex(1 << 20);
    let (sr, sw) = tokio::io::split(server);
    let (_cr, mut cw) = tokio::io::split(client);
    let handler = Blocked::default();
    let (active, peak, gate) = (handler.active.clone(), handler.peak.clone(), handler.gate.clone());
    tokio::spawn(async move {
        if let Ok(running) = handler.serve(Session::new(sr, sw)).await {
            let _ = running.waiting().await;
        }
    });
    let init = r#""id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}"#;
    cw.write_all((line(init) + &line(r#""method":"notifications/initialized""#)).as_bytes()).await?;
    let call =
        |id: usize| line(&format!(r#""id":{id},"method":"tools/call","params":{{"name":"x","arguments":{{}}}}"#));
    let cancel = |id: usize| line(&format!(r#""method":"notifications/cancelled","params":{{"requestId":{id}}}"#));

    let flood: String = (1..=2 * MAX_OUTSTANDING).map(|id| call(id) + &cancel(id)).collect();
    cw.write_all(flood.as_bytes()).await?;
    until(|| active.load(Ordering::SeqCst) >= MAX_OUTSTANDING).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(peak.load(Ordering::SeqCst), MAX_OUTSTANDING, "cancelled requests freed slots early");

    gate.add_permits(2 * MAX_OUTSTANDING);
    until(|| active.load(Ordering::SeqCst) == 0).await?;
    Ok(())
}

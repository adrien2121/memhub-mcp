//! [`Transport`] implementation for [`Session`]: slot accounting on send and receive.

use std::sync::Arc;

use rmcp::RoleServer;
use rmcp::model::{
    ClientNotification, ErrorData, GetExtensions, JsonRpcError, JsonRpcMessage, JsonRpcNotification, JsonRpcResponse,
    RequestId,
};
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use tokio::io::{AsyncRead, AsyncWrite};

use super::{Held, Session, Slot};

/// Removes `id` from the in-flight map and hands back the map's share of its slot.
///
/// # Arguments
/// * `held` - The session's in-flight map; locked briefly.
/// * `id` - The request id being answered or cancelled.
///
/// # Returns
/// The map's share of the slot, or `None` if `id` was not in flight or the lock is poisoned.
fn release(held: &Held, id: &RequestId) -> Option<Slot> {
    held.lock().ok()?.remove(id)
}

impl<R, W> Transport<RoleServer> for Session<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Error = std::io::Error;

    /// Writes one outgoing message; for a response or an error with an id, drops the map's
    /// share of that request's slot once the write completes (successfully or not).
    ///
    /// # Arguments
    /// * `item` - The JSON-RPC message to write to the client.
    ///
    /// # Returns
    /// A `'static` future resolving when the message has been written.
    ///
    /// # Errors
    /// * [`std::io::Error`] - Writing to the client stream failed.
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let slot = match &item {
            JsonRpcMessage::Response(JsonRpcResponse { id, .. })
            | JsonRpcMessage::Error(JsonRpcError { id: Some(id), .. }) => release(&self.held, id),
            _ => None,
        };
        let send = self.inner.send(item);
        async move {
            let sent = send.await;
            drop(slot);
            sent
        }
    }

    /// Reads the next client message, first waiting for a free slot so a session never holds
    /// more than [`super::MAX_OUTSTANDING`] requests that are running or awaiting their reply.
    ///
    /// A slot has two shares: one in the in-flight map, dropped once the reply is written or the
    /// request is cancelled (rmcp never sends a cancelled request's reply), and one in the
    /// request's extensions, dropped by rmcp when the handler ends. The slot frees when both are
    /// gone. A request reusing an in-flight id is answered with an `invalid_request` error and
    /// skipped, keeping the slot. If cancelled while waiting, the slot is kept in `ready`.
    ///
    /// # Returns
    /// The next message to dispatch, or `None` when the stream ends, the semaphore closes, the
    /// in-flight map lock is poisoned, or the duplicate-id error cannot be written.
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        let slot = match self.ready.take() {
            Some(slot) => slot,
            None => Arc::clone(&self.slots).acquire_owned().await.ok()?,
        };
        self.ready = Some(slot);
        loop {
            let mut msg = self.inner.receive().await?;
            match &mut msg {
                JsonRpcMessage::Request(r) => {
                    let admitted = {
                        let Ok(mut held) = self.held.lock() else { return None };
                        let fresh = !held.contains_key(&r.id);
                        if let (true, Some(slot)) = (fresh, self.ready.take()) {
                            let slot = Slot(Arc::new(slot));
                            held.insert(r.id.clone(), slot.clone());
                            r.request.extensions_mut().insert(slot);
                        }
                        fresh
                    };
                    if !admitted {
                        // Reusing an in-flight id would orphan its slot; refuse it, keep the slot.
                        let error = ErrorData::invalid_request("request id is already in flight", None);
                        let reply = JsonRpcMessage::Error(JsonRpcError::new(Some(r.id.clone()), error));
                        self.inner.send(reply).await.ok()?;
                        continue;
                    }
                }
                JsonRpcMessage::Notification(JsonRpcNotification {
                    notification: ClientNotification::CancelledNotification(c),
                    ..
                }) => {
                    if let Some(id) = &c.params.request_id {
                        release(&self.held, id);
                    }
                }
                _ => {}
            }
            return Some(msg);
        }
    }

    /// Closes the underlying stream transport.
    ///
    /// # Returns
    /// `Ok(())` once the transport is closed.
    ///
    /// # Errors
    /// * [`std::io::Error`] - Closing the underlying transport failed.
    async fn close(&mut self) -> Result<(), Self::Error> {
        self.inner.close().await
    }
}

//! Tracks which client requests still await a daemon reply, from newline-delimited JSON-RPC.

use serde_json::Value;

/// Ids of client requests not yet answered, fed raw stream chunks from both directions.
#[derive(Default)]
pub struct Pending {
    ids: Vec<Value>,
    from_client: Vec<u8>,
    from_daemon: Vec<u8>,
}

impl Pending {
    /// Feeds a raw chunk the client sent; every complete message with both `method` and `id`
    /// (a request) adds its id unless already tracked. Partial lines are buffered.
    ///
    /// # Arguments
    /// * `chunk` - Bytes just forwarded from stdin to the daemon; may split or hold several lines.
    pub fn client_sent(&mut self, chunk: &[u8]) {
        for msg in frames(&mut self.from_client, chunk) {
            if msg.get("method").is_some()
                && let Some(id) = msg.get("id")
                && !self.ids.contains(id)
            {
                self.ids.push(id.clone());
            }
        }
    }

    /// Feeds a raw chunk the daemon sent; every complete reply (an `id` with `result` or
    /// `error` and no `method`) stops tracking that id. Partial lines are buffered.
    ///
    /// # Arguments
    /// * `chunk` - Bytes just forwarded from the daemon to stdout.
    pub fn daemon_sent(&mut self, chunk: &[u8]) {
        for msg in frames(&mut self.from_daemon, chunk) {
            let is_reply = msg.get("method").is_none() && (msg.get("result").is_some() || msg.get("error").is_some());
            if let (true, Some(id)) = (is_reply, msg.get("id")) {
                self.ids.retain(|p| p != id);
            }
        }
    }

    /// Reports whether no forwarded request is still awaiting its reply.
    ///
    /// # Returns
    /// `true` when every tracked request has been answered.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// Appends `chunk` to `buf`, removes every complete line, and parses each as JSON, flattening
/// batch arrays into their messages. Malformed lines yield nothing; an unterminated tail stays
/// in `buf` for the next chunk.
///
/// # Arguments
/// * `buf` - One direction's carry-over buffer of incomplete line bytes.
/// * `chunk` - Newly received bytes.
///
/// # Returns
/// The parsed messages from the complete lines, in order.
fn frames(buf: &mut Vec<u8>, chunk: &[u8]) -> Vec<Value> {
    buf.extend_from_slice(chunk);
    let Some(end) = buf.iter().rposition(|&b| b == b'\n') else {
        return Vec::new();
    };
    let lines: Vec<u8> = buf.drain(..=end).collect();
    lines
        .split(|&b| b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .flat_map(|v| match v {
            Value::Array(batch) => batch,
            one => vec![one],
        })
        .collect()
}

#[cfg(test)]
mod tests;

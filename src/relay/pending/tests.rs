//! Tests for request/reply tracking in [`super::Pending`].

use super::Pending;

/// Builds a [`Pending`] fed `client` chunks then `daemon` chunks, in order.
///
/// # Arguments
/// * `client` - Raw chunks the client sent.
/// * `daemon` - Raw chunks the daemon sent.
///
/// # Returns
/// The tracker after all chunks.
fn track(client: &[&str], daemon: &[&str]) -> Pending {
    let mut p = Pending::default();
    client.iter().for_each(|c| p.client_sent(c.as_bytes()));
    daemon.iter().for_each(|d| p.daemon_sent(d.as_bytes()));
    p
}

/// Proves replies arriving in a different order than their requests still clear all of them:
/// answering only id 2 leaves id 1 pending; answering both (one as an error) empties it.
#[test]
fn out_of_order_replies_clear_all_in_flight_requests() {
    let reqs = ["{\"id\":1,\"method\":\"a\"}\n", "{\"id\":2,\"method\":\"b\"}\n"];
    let p = track(&reqs, &["{\"id\":2,\"result\":{}}\n"]);
    assert!(!p.is_empty());
    let p = track(&reqs, &["{\"id\":2,\"result\":{}}\n{\"id\":1,\"error\":{}}\n"]);
    assert!(p.is_empty());
}

/// Proves ids compare by JSON type: a reply with string id `"1"` does not answer request `1`.
#[test]
fn string_and_number_ids_are_distinct() {
    let p = track(&["{\"id\":1,\"method\":\"a\"}\n"], &["{\"id\":\"1\",\"result\":0}\n"]);
    assert!(!p.is_empty());
}

/// Proves a daemon message with a `method` (a server-to-client request) is not a reply, even
/// when its id and a `result` match a pending client request.
#[test]
fn server_request_with_same_id_is_not_a_reply() {
    let p = track(&["{\"id\":7,\"method\":\"a\"}\n"], &["{\"id\":7,\"method\":\"ping\",\"result\":0}\n"]);
    assert!(!p.is_empty());
}

/// Proves notifications (no `id`) and client-sent replies (no `method`) are never tracked.
#[test]
fn notifications_and_client_replies_never_count() {
    let p = track(&["{\"method\":\"n\"}\n{\"id\":3,\"result\":{}}\n"], &["{\"method\":\"n\"}\n"]);
    assert!(p.is_empty());
}

/// Proves a request split across two chunks is tracked once complete, and one chunk carrying
/// two requests tracks both until each gets its reply.
#[test]
fn split_frame_is_tracked_and_chunk_may_hold_two_messages() {
    let mut p = track(&["{\"id\":1,\"met", "hod\":\"a\"}\n{\"id\":2,\"method\":\"b\"}\n"], &[]);
    p.daemon_sent(b"{\"id\":2,\"result\":0}\n");
    assert!(!p.is_empty());
    p.daemon_sent(b"{\"id\":1,\"result\":0}\n");
    assert!(p.is_empty());
}

/// Proves a reply split across two daemon chunks clears its request once the line completes.
#[test]
fn split_reply_is_tracked() {
    let mut p = track(&["{\"id\":1,\"method\":\"a\"}\n"], &["{\"id\":1,\"res"]);
    p.daemon_sent(b"ult\":0}\n");
    assert!(p.is_empty());
}

/// Proves JSON-RPC batch arrays are tracked per message on both sides and malformed lines
/// are skipped without disturbing the rest of the chunk.
#[test]
fn batches_count_and_malformed_lines_are_ignored() {
    let mut p = track(&["not json\n[{\"id\":1,\"method\":\"a\"},{\"id\":2,\"method\":\"b\"}]\n"], &[]);
    p.daemon_sent(b"{oops\n[{\"id\":1,\"result\":0}]\n");
    assert!(!p.is_empty());
    p.daemon_sent(b"[{\"id\":2,\"result\":0}]\n");
    assert!(p.is_empty());
}

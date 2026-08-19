//! Tests for the client handle (#189 Phase 11).

use super::*;

use std::sync::Arc;

fn peer() -> (Peer, mpsc::Receiver<String>) {
    let (tx, rx) = mpsc::channel(16);
    (Peer::new(tx), rx)
}

fn sent(rx: &mut mpsc::Receiver<String>) -> serde_json::Value {
    let line = rx.try_recv().expect("something was sent");
    serde_json::from_str(&line).expect("what is sent is JSON")
}

#[test]
fn an_update_goes_out_as_a_session_update_notification() {
    let (peer, mut rx) = peer();

    peer.update(
        "sess_1",
        SessionUpdate::AgentMessageChunk {
            content: crate::protocol::ContentBlock::text("hi"),
        },
    );

    let value = sent(&mut rx);
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["method"], "session/update");
    assert_eq!(value["params"]["sessionId"], "sess_1");
    assert!(
        value.get("id").is_none(),
        "a notification must carry no id: {value}"
    );
}

/// The three options an editor shows. `allow_always` is offered but is not a
/// standing grant here — see the constructor's note.
#[tokio::test]
async fn a_permission_request_offers_allow_and_reject() {
    let (peer, mut rx) = peer();

    let asking = tokio::spawn(async move {
        // Never answered; the point is what goes out.
        let _ = peer
            .request_permission("sess_1", "call_1", "Run cargo test")
            .await;
    });
    tokio::task::yield_now().await;

    let value = sent(&mut rx);
    assert_eq!(value["method"], "session/request_permission");
    assert!(value["id"].is_number(), "a question needs an id: {value}");
    assert_eq!(value["params"]["toolCall"]["title"], "Run cargo test");

    let kinds: Vec<&str> = value["params"]["options"]
        .as_array()
        .expect("options")
        .iter()
        .filter_map(|option| option["kind"].as_str())
        .collect();
    assert_eq!(kinds, ["allow_once", "allow_always", "reject_once"]);

    asking.abort();
}

/// Every answer that is not an explicit allow is a refusal. This is the
/// property the whole permission path rests on, so it is asserted directly
/// rather than inferred from the happy case.
#[tokio::test]
async fn anything_other_than_an_allow_is_refused() {
    for reply in [
        serde_json::json!({"outcome": {"outcome": "selected", "optionId": "reject-once"}}),
        serde_json::json!({"outcome": {"outcome": "cancelled"}}),
        // An option id nobody offered.
        serde_json::json!({"outcome": {"outcome": "selected", "optionId": "invented"}}),
        // A shape that does not parse at all.
        serde_json::json!({"nonsense": true}),
    ] {
        let (tx, mut rx) = mpsc::channel(16);
        let peer = Arc::new(Peer::new(tx));
        let asking = {
            let peer = Arc::clone(&peer);
            tokio::spawn(async move { peer.request_permission("s", "c", "t").await })
        };
        tokio::task::yield_now().await;

        let request = sent(&mut rx);
        peer.resolve(
            serde_json::from_value(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": reply,
            }))
            .expect("the reply parses as an incoming message"),
        );

        assert!(
            !asking.await.expect("the ask completes"),
            "this reply should not have granted permission: {reply}"
        );
    }
}

#[tokio::test]
async fn an_allow_is_granted() {
    let (tx, mut rx) = mpsc::channel(16);
    let peer = Arc::new(Peer::new(tx));
    let asking = {
        let peer = Arc::clone(&peer);
        tokio::spawn(async move { peer.request_permission("s", "c", "t").await })
    };
    tokio::task::yield_now().await;

    let request = sent(&mut rx);
    peer.resolve(
        serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {"outcome": {"outcome": "selected", "optionId": ALLOW_ONCE}},
        }))
        .expect("parses"),
    );

    assert!(asking.await.expect("completes"));
}

/// A disconnected client must not leave the turn waiting forever.
///
/// Dropping the outer `Arc` cannot do it: the waiting turn holds one of its
/// own, and the sender that would wake it lives inside the very `Peer` it is
/// holding. `disconnect` is what breaks that cycle, and the serve loop calls it
/// before waiting for in-flight turns for exactly this reason.
#[tokio::test]
async fn a_disconnected_client_resolves_a_pending_question_to_a_refusal() {
    let (tx, _rx) = mpsc::channel(16);
    let peer = Arc::new(Peer::new(tx));
    let asking = {
        let peer = Arc::clone(&peer);
        tokio::spawn(async move { peer.request_permission("s", "c", "t").await })
    };
    tokio::task::yield_now().await;

    peer.disconnect();

    let granted = tokio::time::timeout(std::time::Duration::from_secs(5), asking)
        .await
        .expect("the turn must not hang waiting for a client that has gone")
        .expect("completes");
    assert!(!granted, "an unanswered question must resolve to a refusal");
}

#[test]
fn a_reply_for_nothing_that_was_asked_is_ignored_rather_than_fatal() {
    let (peer, _rx) = peer();

    peer.resolve(
        serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 999,
            "result": {},
        }))
        .expect("parses"),
    );
}

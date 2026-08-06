// The drain gate, driven through the real terminal path.
//
// The policy is covered in `lifecycle_policy::drain_gate`; what these assert is
// that the gate actually sits between the final acceptance gate and the
// accepted report — a correct policy nobody calls is the failure mode this
// feature was written against.

use std::sync::Arc;

use serde_json::Value;

use crate::board_port::{DrainItem, DrainItemKind, DrainStatus, WorkflowBoardPort};

use super::LifecycleEvidence;
use super::review_test_host::{RecordingHost, accepted_report, driver, preamble_reply};

struct StubBoard(Vec<DrainItem>);

impl WorkflowBoardPort for StubBoard {
    fn drain_items_for_run(&self, _run_id: &str) -> crate::WorkflowResult<Vec<DrainItem>> {
        Ok(self.0.clone())
    }
}

fn issue(id: &str, status: DrainStatus, decline_reason: Option<&str>) -> DrainItem {
    DrainItem {
        id: id.to_string(),
        title: format!("{id} needs a decision"),
        kind: DrainItemKind::Issue,
        status,
        decline_reason: decline_reason.map(str::to_string),
    }
}

fn clean_run_responder(method: &str, id: &str) -> Value {
    if let Some(reply) = preamble_reply(id) {
        return reply;
    }
    if id.starts_with("cross-cutting-review-")
        || id.starts_with("final-evidence-reconciliation-")
        || id == "require-final-artifacts"
        || id == "final-zero-gap-audit"
    {
        return serde_json::json!({ "status": "accepted", "items": [] });
    }
    if id == "final-acceptance-gate" {
        return serde_json::json!({ "status": "accepted" });
    }
    if method == "finalReport" {
        return accepted_report();
    }
    panic!("unexpected host call: {method} {id}");
}

async fn run_with_board(items: Vec<DrainItem>) -> Arc<RecordingHost> {
    let host = RecordingHost::new(Box::new(clean_run_responder));
    let driver = driver(host.clone()).with_board_drain("wf-11111111", Arc::new(StubBoard(items)));
    let mut evidence = LifecycleEvidence::default();
    driver
        .run_review_and_final_gates(&serde_json::json!({ "items": [] }), &mut evidence)
        .await
        .expect("review gates run to a report");
    host
}

#[tokio::test]
async fn an_open_board_item_fails_the_run() {
    let host = run_with_board(vec![
        issue("board-1", DrainStatus::Resolved, None),
        issue("board-2", DrainStatus::Open, None),
    ])
    .await;

    let ids = host.call_ids();
    assert!(ids.contains(&"blocked-board-drain".to_string()), "{ids:?}");
    assert!(
        !ids.contains(&"final-acceptance-report".to_string()),
        "{ids:?}"
    );
}

#[tokio::test]
async fn a_fully_drained_board_reaches_acceptance() {
    let host = run_with_board(vec![
        issue("board-1", DrainStatus::Resolved, None),
        issue("board-2", DrainStatus::Promoted, None),
        issue(
            "board-3",
            DrainStatus::Declined,
            Some("superseded by the write-coordinator rewrite"),
        ),
    ])
    .await;

    let ids = host.call_ids();
    assert!(
        ids.contains(&"final-acceptance-report".to_string()),
        "{ids:?}"
    );
    assert!(!ids.contains(&"blocked-board-drain".to_string()), "{ids:?}");
}

#[tokio::test]
async fn a_decline_without_a_reason_fails_the_run() {
    let host = run_with_board(vec![issue("board-1", DrainStatus::Declined, None)]).await;

    let ids = host.call_ids();
    assert!(ids.contains(&"blocked-board-drain".to_string()), "{ids:?}");
    assert!(
        !ids.contains(&"final-acceptance-report".to_string()),
        "{ids:?}"
    );
}

#[tokio::test]
async fn a_run_without_a_board_still_reaches_acceptance() {
    let host = RecordingHost::new(Box::new(clean_run_responder));
    let driver = driver(host.clone());
    let mut evidence = LifecycleEvidence::default();
    driver
        .run_review_and_final_gates(&serde_json::json!({ "items": [] }), &mut evidence)
        .await
        .expect("review gates run to a report");

    assert!(
        host.call_ids()
            .contains(&"final-acceptance-report".to_string())
    );
}

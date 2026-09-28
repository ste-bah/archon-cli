//! Issue-128: an acceptance check that could not be evaluated is never sent
//! to its tasks as "fix the implementation".
use super::tests::{ACCEPTANCE_TAIL, acceptance_reply, run_scripted, script, view};

/// The live shape: one scratch collision copied onto every check as
/// `status: "error"`, each owned. Nothing is remediated; the loop ends and
/// the gate stays incomplete. A check that RAN and failed in the same round
/// still goes to its owner.
#[tokio::test]
async fn an_erroring_check_is_never_routed_to_its_tasks() {
    let (calls, result) = run_scripted(&script("schema: 2, ", ACCEPTANCE_TAIL), |_, payload| {
        let id = payload["id"].as_str().unwrap_or_default();
        if id == "acceptance-contract-run-1" {
            return acceptance_reply(
                1,
                serde_json::json!([
                    { "check_id": "REQ-1", "criterion": "one", "kind": "command", "status": "error",
                      "exit_code": null, "operational_error": "nonidentical scratch path collision",
                      "owning_tasks": ["TASK-Q-001"] },
                    { "check_id": "REQ-2", "criterion": "two", "kind": "command", "status": "error",
                      "exit_code": null, "operational_error": "nonidentical scratch path collision",
                      "owning_tasks": ["TASK-Q-002"],
                      "routing": {"implicated_files": ["src/two.txt"], "writer_tasks": ["TASK-Q-001"], "granted_files": []} }
                ]),
                false,
            );
        }
        if id.starts_with("acceptance-contract-run-") {
            return acceptance_reply(2, serde_json::json!([]), true);
        }
        view(serde_json::json!({ "items": [], "outcomes": [] }), "accepted")
    })
    .await;
    assert!(
        !calls.iter().any(|(_, p)| p["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("review-remediate-"))),
        "an unevaluated check reached a task: {calls:#?}"
    );
    assert!(
        !calls
            .iter()
            .any(|(_, p)| p["id"] == "acceptance-contract-run-2"),
        "nothing was remediated, so there is no second round"
    );
    let result: serde_json::Value = serde_json::from_str(&result).expect("accounting json");
    assert_eq!(result["acceptance_gate"]["complete"], false);

    let (calls, _) = run_scripted(&script("schema: 2, ", ACCEPTANCE_TAIL), |_, payload| {
        let id = payload["id"].as_str().unwrap_or_default();
        if id == "acceptance-contract-run-1" {
            return acceptance_reply(
                1,
                serde_json::json!([
                    { "check_id": "REQ-1", "criterion": "one", "kind": "command", "status": "error",
                      "exit_code": null, "owning_tasks": ["TASK-Q-001"] },
                    { "check_id": "REQ-2", "criterion": "two", "kind": "command", "status": "failed",
                      "exit_code": 1, "owning_tasks": ["TASK-Q-002"] }
                ]),
                false,
            );
        }
        if id.starts_with("acceptance-contract-run-") {
            return acceptance_reply(2, serde_json::json!([]), true);
        }
        view(serde_json::json!({ "items": [], "outcomes": [] }), "accepted")
    })
    .await;
    let units: Vec<&str> = calls
        .iter()
        .filter(|(method, p)| {
            method == "fanout"
                && p["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("review-remediate-"))
        })
        .map(|(_, p)| p["id"].as_str().unwrap())
        .collect();
    assert!(
        !units.is_empty() && units.iter().all(|id| id.contains("task-q-002")),
        "only the failed check's owner is remediated: {units:?}"
    );
}

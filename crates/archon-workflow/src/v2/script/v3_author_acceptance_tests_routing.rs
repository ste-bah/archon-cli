//! The acceptance stage loop, continued: regressions routed to the landing
//! that broke them, and the rest of the routing cases.

use super::*;

/// Issue-114: a failing check the host showed regressed at a run landing is
/// routed to that landing's tasks as well as its owners, and the finding
/// says which landing broke it.
#[tokio::test]
async fn a_regressed_check_goes_to_the_landing_that_broke_it_too() {
    let (calls, _) = run_scripted(&script("schema: 2, ", ACCEPTANCE_TAIL), |_, payload| {
        let id = payload["id"].as_str().unwrap_or_default();
        if id == "acceptance-contract-run-1" {
            return acceptance_reply(
                1,
                serde_json::json!([{ "check_id": "REQ-1", "criterion": "one is done", "kind": "command",
                    "status": "failed", "exit_code": 1, "owning_tasks": ["TASK-Q-001"],
                    "regressed_by": {"held_at": "aaa", "landing_commit": "bbb",
                        "landing_stage": "review-remediate-task-q-002-1-9", "tasks": ["TASK-Q-002"]} }]),
                false,
            );
        }
        if id.starts_with("acceptance-contract-run-") {
            return acceptance_reply(2, serde_json::json!([]), true);
        }
        view(serde_json::json!({ "items": [], "outcomes": [] }), "accepted")
    })
    .await;
    let writes: Vec<&serde_json::Value> = calls
        .iter()
        .filter(|(method, p)| {
            method == "fanout" && p["id"].as_str().unwrap().starts_with("review-remediate-")
        })
        .map(|(_, p)| &p["source"][0])
        .collect();
    let tasks: Vec<&serde_json::Value> = writes
        .iter()
        .map(|item| &item["canonical_task_ids"])
        .collect();
    // One unit over the owner and the landing that broke it.
    assert_eq!(
        tasks,
        [&serde_json::json!(["TASK-Q-001", "TASK-Q-002"])],
        "{tasks:?}"
    );
    let prompt = writes[0]["task"].as_str().unwrap();
    assert!(
        prompt.contains("REGRESSION") && prompt.contains("review-remediate-task-q-002-1-9"),
        "{prompt}"
    );
}

/// Batch E: a check whose failure implicates another task's file and a file
/// no task declares goes to one unit over the implementer AND that file's
/// writer, with the unowned file among the unit's targets.
#[tokio::test]
async fn a_check_goes_to_the_writers_of_the_files_it_implicates() {
    let (calls, _) = run_scripted(&script("schema: 2, ", ACCEPTANCE_TAIL), |_, payload| {
        let id = payload["id"].as_str().unwrap_or_default();
        if id == "acceptance-contract-run-1" {
            return acceptance_reply(
                1,
                serde_json::json!([{ "check_id": "REQ-1", "criterion": "one is done", "kind": "command",
                    "status": "failed", "exit_code": 1, "owning_tasks": ["TASK-Q-001"],
                    "routing": {"implicated_files": ["src/two.txt", "src/loose.txt"],
                        "writer_tasks": ["TASK-Q-002"], "granted_files": ["src/loose.txt"]} }]),
                false,
            );
        }
        if id.starts_with("acceptance-contract-run-") {
            return acceptance_reply(2, serde_json::json!([]), true);
        }
        view(serde_json::json!({ "items": [], "outcomes": [] }), "accepted")
    })
    .await;
    let writes: Vec<&serde_json::Value> = calls
        .iter()
        .filter(|(method, p)| {
            method == "fanout" && p["id"].as_str().unwrap().starts_with("review-remediate-")
        })
        .map(|(_, p)| &p["source"][0])
        .collect();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(
        writes[0]["canonical_task_ids"],
        serde_json::json!(["TASK-Q-001", "TASK-Q-002"])
    );
    assert_eq!(
        writes[0]["target_files"],
        serde_json::json!(["src/one.txt", "src/two.txt", "src/loose.txt"])
    );
    let prompt = writes[0]["task"].as_str().unwrap();
    assert!(
        prompt.contains("IMPLICATED FILES: src/two.txt, src/loose.txt"),
        "{prompt}"
    );
}

//! Batch J: the acceptance loop sends a regression to the task whose landing
//! broke it, with the landing's files the host granted; tells an owner what
//! the regression search found when no landing was pinned; and never sends
//! a check the host marked `blocked` to any unit.

use super::tests::{ACCEPTANCE_TAIL, acceptance_reply, run_scripted, script, view};

fn remediation_writes(calls: &[(String, serde_json::Value)]) -> Vec<serde_json::Value> {
    calls
        .iter()
        .filter(|(method, p)| {
            method == "fanout" && p["id"].as_str().unwrap().starts_with("review-remediate-")
        })
        .map(|(_, p)| p["source"][0].clone())
        .collect()
}

#[tokio::test]
async fn a_regression_goes_to_its_author_with_the_grant_and_a_blocked_check_goes_nowhere() {
    let (calls, result) = run_scripted(&script("schema: 2, ", ACCEPTANCE_TAIL), |_, payload| {
        let id = payload["id"].as_str().unwrap_or_default();
        if id == "acceptance-contract-run-1" {
            return acceptance_reply(
                1,
                serde_json::json!([
                    { "check_id": "REQ-1", "criterion": "ingest accepts an unknown class", "kind": "command",
                      "status": "failed", "exit_code": 1, "owning_tasks": ["TASK-Q-001"],
                      "stderr_tail": "Error: unknown asset_class `unknown`",
                      "regressed_by": {"held_at": "aaa", "landing_commit": "bbb",
                          "landing_stage": "review-remediate-task-q-002-1-9", "tasks": ["TASK-Q-002"],
                          "changed_files": ["src/cmd/ingest.rs"], "probed_as": "REQ-0"},
                      "routing": {"implicated_files": ["src/cmd/ingest.rs"],
                          "granted_files": ["src/cmd/ingest.rs"]} },
                    { "check_id": "REQ-2", "criterion": "never works", "kind": "command",
                      "status": "failed", "exit_code": 1, "owning_tasks": ["TASK-Q-001"],
                      "blocked": "it never held at any point of the run, and every file its failure implicates is one no unit may be given" },
                ]),
                false,
            );
        }
        if id.starts_with("acceptance-contract-run-") {
            // The blocked check still fails; the host ends the loop.
            return acceptance_reply(
                2,
                serde_json::json!([{ "check_id": "REQ-2", "criterion": "never works",
                    "kind": "command", "status": "failed", "exit_code": 1,
                    "owning_tasks": ["TASK-Q-001"], "blocked": "no unit can fix it" }]),
                true,
            );
        }
        view(serde_json::json!({ "items": [], "outcomes": [] }), "accepted")
    })
    .await;
    let writes = remediation_writes(&calls);
    // One unit: the owner and the author of the landing that broke it.
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(
        writes[0]["canonical_task_ids"],
        serde_json::json!(["TASK-Q-001", "TASK-Q-002"])
    );
    // The landing's file no task declares is the unit's to write.
    let targets = writes[0]["target_files"].as_array().unwrap();
    assert!(
        targets.contains(&serde_json::json!("src/cmd/ingest.rs")),
        "{targets:?}"
    );
    let prompt = writes[0]["task"].as_str().unwrap();
    assert!(
        prompt.contains("REGRESSION: it held at aaa and first failed at run landing bbb")
            && prompt.contains("landed by TASK-Q-002 (found by probing REQ-0")
            && prompt.contains("restore it in that change"),
        "{prompt}"
    );
    // The blocked check is sent to no unit at all.
    assert!(!prompt.contains("REQ-2"), "{prompt}");
    let result: serde_json::Value = serde_json::from_str(&result).expect("accounting json");
    let blocked = &result["acceptance_gate"]["blocked"];
    assert_eq!(blocked[0]["check_id"], "REQ-2", "{result}");
}

#[tokio::test]
async fn an_unattributed_check_goes_to_its_owner_with_the_search_note() {
    let (calls, _) = run_scripted(&script("schema: 2, ", ACCEPTANCE_TAIL), |_, payload| {
        let id = payload["id"].as_str().unwrap_or_default();
        if id == "acceptance-contract-run-1" {
            return acceptance_reply(
                1,
                serde_json::json!([{ "check_id": "REQ-1", "criterion": "one is done", "kind": "command",
                    "status": "failed", "exit_code": 1, "owning_tasks": ["TASK-Q-001"],
                    "regression_search": {"observed": 3, "points": 9,
                        "note": "the regression search budget (32 observations, 90 min) ran out"} }]),
                false,
            );
        }
        if id.starts_with("acceptance-contract-run-") {
            return acceptance_reply(2, serde_json::json!([]), true);
        }
        view(serde_json::json!({ "items": [], "outcomes": [] }), "accepted")
    })
    .await;
    let writes = remediation_writes(&calls);
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(
        writes[0]["canonical_task_ids"],
        serde_json::json!(["TASK-Q-001"])
    );
    let prompt = writes[0]["task"].as_str().unwrap();
    assert!(
        prompt.contains(
            "REGRESSION SEARCH: the regression search budget (32 observations, 90 min) ran out."
        ),
        "{prompt}"
    );
}

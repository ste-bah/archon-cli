//! REM-16: the review map's read-only shell grant and what its findings say
//! they ran.

use std::collections::BTreeMap;

use serde_json::json;

use super::super::attributed_map_findings;
use super::*;
use crate::stage_command_policy::command_execution_stage;
use crate::v2::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};
use crate::{ProviderTier, StageKind, StageRunRequest};

fn item(id: &str) -> WorkflowV2FanoutItem {
    let call = WorkflowV2HostCall {
        id: "adversarial-review-map".to_string(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    WorkflowV2FanoutItem::read_only(
        id,
        "critic",
        call,
        json!({ "item": { "item_id": id, "canonical_task_ids": ["TASK-A"] } }),
    )
}

/// The stage request the live client builds from a branch input.
fn stage_request(input: &Value) -> StageRunRequest {
    let mut input = input.clone();
    input["v2_call"] = json!({ "id": "adversarial-review-map-0", "method": "parallel",
        "role": "critic", "write_mode": null, "target_files": [] });
    StageRunRequest {
        run_id: "run".into(),
        stage_id: "adversarial-review-map-0".into(),
        stage_kind: StageKind::Fanout,
        agent: Some("critic".into()),
        task: "review".into(),
        attempt: 1,
        provider_tier: ProviderTier::Critic,
        depends_on: Vec::new(),
        input,
    }
}

#[test]
fn a_granted_review_branch_is_a_command_stage_and_keeps_its_recorded_identity() {
    // As live: the host stamped its artifact policy before the split.
    let mut plain = item("adversarial-review-map-0");
    plain.input["_workflow_project_artifact_policy"] = json!({"root": "/here"});
    assert!(
        !command_execution_stage(&stage_request(&plain.input)),
        "without the grant a review branch has no shell, as before"
    );
    let mut items = vec![plain.clone()];
    let identities = grant_review_commands(&mut items, true);
    assert!(command_execution_stage(&stage_request(&items[0].input)));
    assert_eq!(
        items[0].input[REVIEW_EXECUTION_INPUT_KEY],
        json!(REVIEW_EXECUTION_RULE)
    );
    // Granting twice adds nothing.
    let mut again = items.clone();
    grant_review_commands(&mut again, true);
    assert_eq!(
        again[0].input["stage_extra"]["allowed_tools"],
        json!(["Bash"])
    );
    // The outcome is filed under the branch as it was before the stamp, so a
    // resume -- which splits unstamped items -- matches it.
    let mut outcome = WorkflowV2BranchOutcome {
        item_id: plain.id.clone(),
        role: "critic".into(),
        status: crate::v2::WorkflowV2Status::Accepted,
        result: None,
        error: None,
        failure_kind: None,
        item_input_hash: Some(items[0].input_hash()),
        completion_evidence: Vec::new(),
    };
    restore_review_identity(&mut outcome, &identities);
    // Minor 6: filed under the reuse identity, the item as authored, which
    // a host stamp (here the artifact policy) never moves.
    // A resume whose host stamps the policy differently still matches it.
    let mut resumed = plain.clone();
    resumed.input["_workflow_project_artifact_policy"] = json!({"root": "/elsewhere"});
    let recorded = outcome.item_input_hash.clone().expect("filed");
    assert!(crate::v2::reuse_identity::recorded_hash_matches(
        &recorded, &resumed
    ));
}

fn map_data(marked: bool) -> Value {
    let mut result = json!({
        "status": "needs_review", "summary": "s",
        "commands_run": [
            { "kind": "test", "command": "cargo test -p a --test t focused", "status": "failed", "exit_code": 101, "output_summary": "1 failed" },
            { "kind": "inspect", "command": "rg -n retry crates/a", "status": "succeeded", "exit_code": 0, "output_summary": "" },
        ],
        "data": { "findings": [ { "id": "F1", "claim": "the focused test fails" } ] },
    });
    if marked {
        let mut wrapped = WorkflowV2Result::accepted("x");
        mark_review_command_access(&mut wrapped, true);
        result["data"][REVIEW_COMMAND_ACCESS_KEY] = wrapped.data[REVIEW_COMMAND_ACCESS_KEY].clone();
    }
    json!({ "outcomes": [ { "item_id": "adversarial-review-map-0", "status": "needs_review",
        "canonical_task_ids": ["TASK-A"], "result": result } ] })
}

#[test]
fn every_finding_of_a_granted_branch_names_the_commands_it_ran() {
    let found = attributed_map_findings(&map_data(true), &BTreeMap::new());
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0][REVIEW_COMMANDS_RUN_KEY],
        json!([
            "cargo test -p a --test t focused (exit 101)",
            "rg -n retry crates/a (exit 0)"
        ])
    );
}

#[test]
fn a_branch_recorded_without_the_grant_keeps_its_findings_exactly() {
    let found = attributed_map_findings(&map_data(false), &BTreeMap::new());
    assert_eq!(found.len(), 1);
    assert!(
        found[0].get(REVIEW_COMMANDS_RUN_KEY).is_none(),
        "no stamp, so an earlier run's host finding ids never move: {}",
        found[0]
    );
}

/// REM-16: the prompt's rules read the same grant the tool list does.
#[test]
fn the_prompt_rules_follow_the_command_grant() {
    use crate::stage_command_policy::v2_call_runs_commands;
    let plain = item("adversarial-review-map-0");
    assert!(!v2_call_runs_commands(
        "adversarial-review-map-0",
        &plain.input
    ));
    let mut granted = vec![plain];
    grant_review_commands(&mut granted, true);
    assert!(v2_call_runs_commands(
        "adversarial-review-map-0",
        &granted[0].input
    ));
    assert!(v2_call_runs_commands(
        "verification-wave-verify-task-a-2",
        &json!({})
    ));
    assert!(!v2_call_runs_commands(
        "adversarial-review-reduce",
        &json!({})
    ));
}

/// Major 2: a branch that ran with no shell says so on every finding.
#[test]
fn a_branch_that_had_no_shell_says_it_ran_nothing() {
    let mut data = map_data(false);
    let mut wrapped = WorkflowV2Result::accepted("x");
    mark_review_command_access(&mut wrapped, false);
    data["outcomes"][0]["result"]["data"][REVIEW_COMMAND_ACCESS_KEY] =
        wrapped.data[REVIEW_COMMAND_ACCESS_KEY].clone();
    let found = attributed_map_findings(&data, &BTreeMap::new());
    assert_eq!(found[0][REVIEW_COMMANDS_RUN_KEY], json!([]));
}

/// The late review kinds the host reads are the ones the prelude sends.
#[test]
fn the_prelude_sends_the_late_review_kinds_the_host_names() {
    let prelude = include_str!("script/v3_primitives.js");
    for kind in [ADVERSARIAL_MOVED_KIND, COVERAGE_MOVED_KIND] {
        assert!(prelude.contains(&format!("\"{kind}\"")), "{kind}");
    }
}

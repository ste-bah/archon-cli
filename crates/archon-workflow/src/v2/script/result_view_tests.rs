//! The envelope a v3 script is handed: each branch result once. The compat
//! shape the decomposed driver reads stays whole.

use super::*;
use crate::v2::call_data::result_from_fanout_report;
use crate::{
    WorkflowV2BranchOutcome, WorkflowV2FanoutReport, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2Status,
};

const SENTINEL: &str = "BRANCH_RESULT_SENTINEL_9c1e";

fn deduped(result: &WorkflowV2Result) -> WorkflowResult<String> {
    result_view_json_shaped(result, ScriptEnvelopeShape::Deduped)
}

fn read_only_fanout(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: None,
        options: Default::default(),
    }
}

fn branch_outcome(item_id: &str, result: WorkflowV2Result) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: item_id.to_string(),
        role: "coder".to_string(),
        status: result.status,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

/// A real verifier branch outcome, stamped with a sentinel that lives only
/// inside the branch result's own `data` — so every copy of the result, and
/// nothing else, carries it.
fn verifier_branch() -> WorkflowV2BranchOutcome {
    let mut outcome: WorkflowV2BranchOutcome =
        serde_json::from_str(archon_test_support::fixtures::WFCD824_VERIFICATION_WAVE_1_3_CHECK_1)
            .expect("fixture");
    let result = outcome.result.as_mut().expect("fixture carries a result");
    result.data["branch_sentinel"] = serde_json::Value::String(SENTINEL.to_string());
    outcome
}

fn single_branch_fanout(outcome: WorkflowV2BranchOutcome) -> WorkflowV2Result {
    let normalized = result_from_fanout_report(
        &read_only_fanout("verification-wave-verify-task-1"),
        WorkflowV2FanoutReport {
            outcomes: vec![outcome],
            max_parallelism: 1,
            peak_parallelism: 1,
            cancelled: false,
        },
    );
    let mut result = normalized.result;
    result.data["branch_artifact_paths"] = serde_json::json!(["/run/v2/branches/b.json"]);
    result
}

/// The compat shape is what every script used to receive; the ratio against
/// it is the measured saving.
fn legacy_view_len(result: &WorkflowV2Result) -> usize {
    result_view_json(result).expect("compat view").len()
}

#[test]
fn fanout_envelope_carries_each_branch_result_exactly_once() {
    let result = single_branch_fanout(verifier_branch());
    // The persisted record keeps both copies the host's own readers use
    // (`data.items[0]` and `data.outcomes[0].result`); only the view changes.
    assert_eq!(serde_json::to_string(&result).unwrap().matches(SENTINEL).count(), 2);

    let json = deduped(&result).expect("view");
    assert_eq!(json.matches(SENTINEL).count(), 1, "{json}");

    let view: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(view["items"][0], result.data["items"][0], "items[0] is the canonical copy");
    assert!(view["outcomes"][0].get("result").is_none(), "{}", view["outcomes"][0]);
    assert_eq!(view["outcomes"][0]["item_id"], result.data["outcomes"][0]["item_id"]);
    assert!(view["result"]["data"].get("items").is_none());
    assert!(view["result"]["data"].get("outcomes").is_none());
    // Every other data key survives in both places.
    assert_eq!(view["peak_parallelism"], 1);
    assert_eq!(view["result"]["data"]["peak_parallelism"], 1);
    assert_eq!(view["result"]["data"]["branch_artifact_paths"][0], "/run/v2/branches/b.json");
    assert_eq!(view["status"], view["result"]["status"]);
    assert_eq!(view["summary"], view["result"]["summary"]);
    // The lifted branch evidence the prelude's `usable(env)` reads stays.
    assert!(view["result"]["commands_run"].as_array().is_some_and(|c| !c.is_empty()));

    let legacy = legacy_view_len(&result);
    assert!(json.len() * 2 < legacy, "view {} bytes vs legacy {legacy}", json.len());
}

#[test]
fn the_compat_shape_keeps_every_copy_the_decomposed_driver_reads() {
    let result = single_branch_fanout(verifier_branch());
    let json = result_view_json(&result).expect("view");
    assert_eq!(json.matches(SENTINEL).count(), 4, "{json}");
    let view: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(view["outcomes"][0]["result"].is_object());
    assert!(view["result"]["data"]["items"].is_array());
    assert!(view["result"]["data"]["outcomes"].is_array());
}

#[test]
fn only_meta_marked_scripts_get_the_deduplicated_envelope() {
    assert_eq!(
        script_envelope_shape("export const meta = { name: 'x' }\nphase('a')\n"),
        ScriptEnvelopeShape::Deduped
    );
    assert_eq!(
        script_envelope_shape("async function workflow(w) { return await w.agent('a', {}) }"),
        ScriptEnvelopeShape::Compat
    );
}

#[test]
fn an_outcome_result_absent_from_items_is_kept() {
    let mut result = single_branch_fanout(verifier_branch());
    // Simulate an outcome whose result the items array does not carry.
    result.data["outcomes"][0]["result"]["summary"] = "diverged".into();
    let view: serde_json::Value =
        serde_json::from_str(&deduped(&result).expect("view")).unwrap();
    assert_eq!(view["outcomes"][0]["result"]["summary"], "diverged");
}

#[test]
fn write_fanout_items_appear_once_in_the_view() {
    // Write fan-outs already emit outcomes without a nested result; their
    // duplicate was the nested `result.data.items`.
    let mut branch = WorkflowV2Result::accepted("branch done");
    branch.data = serde_json::json!({ "item_id": "impl-1", "branch_sentinel": SENTINEL });
    let mut aggregate = WorkflowV2Result::accepted("write fanout done");
    aggregate.data = serde_json::json!({
        "items": [branch],
        "outcomes": [{ "item_id": "impl-1", "status": "accepted", "contract_valid": true }],
        "write_mode": "worktree",
        "peak_parallelism": 1,
    });
    let json = deduped(&aggregate).expect("view");
    assert_eq!(json.matches(SENTINEL).count(), 1, "{json}");
    let view: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(view["outcomes"][0]["contract_valid"], true);
    assert_eq!(view["result"]["data"]["write_mode"], "worktree");
}

#[test]
fn non_fanout_data_keeps_the_nested_copy_the_decomposition_script_compares() {
    // `workflow_decompose_v1.js` checks `outcome.publicationReceipt.call_id`
    // against `outcome.result.data.publicationReceipt.call_id`.
    let mut result = WorkflowV2Result::accepted("landed");
    result.data = serde_json::json!({ "publicationReceipt": { "call_id": "phase-e-1" } });
    let view: serde_json::Value =
        serde_json::from_str(&deduped(&result).expect("view")).unwrap();
    assert_eq!(view["publicationReceipt"]["call_id"], "phase-e-1");
    assert_eq!(view["result"]["data"]["publicationReceipt"]["call_id"], "phase-e-1");
}

#[test]
fn scalar_data_is_carried_under_data_unchanged() {
    let mut result = WorkflowV2Result::accepted("text");
    result.data = serde_json::Value::String("raw".to_string());
    let view: serde_json::Value =
        serde_json::from_str(&deduped(&result).expect("view")).unwrap();
    assert_eq!(view["data"], "raw");
    assert_eq!(view["result"]["data"], "raw");
}

#[test]
fn a_blocked_branch_result_appears_once() {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Blocked,
        summary: "blocked".to_string(),
        ..WorkflowV2Result::default()
    };
    result.data = serde_json::json!({ "branch_sentinel": SENTINEL });
    let outcome = branch_outcome("b-1", result);
    let fanout = single_branch_fanout(outcome);
    let json = deduped(&fanout).expect("view");
    assert_eq!(json.matches(SENTINEL).count(), 1, "{json}");
}

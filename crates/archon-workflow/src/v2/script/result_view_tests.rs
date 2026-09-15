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

// ---- Issue-19: top-level mirrors of the typed evidence arrays -----------------

/// The live wf-719ff3b0 verify result: accepted, 21 commands under `result`.
fn live_verify_result() -> WorkflowV2Result {
    serde_json::from_str(
        archon_test_support::fixtures::WF719F_VERIFICATION_WAVE_VERIFY_TASK_DL_001_16_RESULT,
    )
    .expect("live verify result deserializes")
}

/// The authored script's own `isAccepted`, cut from the recorded script text
/// (from its `function isAccepted(env) {` line to the closing brace).
fn live_is_accepted_snippet() -> String {
    let script = archon_test_support::fixtures::WF719F_AUTHORED_WORKFLOW_JS;
    let start = script
        .find("function isAccepted(env) {")
        .expect("the live script defines isAccepted");
    let end = start + script[start..].find("\n}\n").expect("isAccepted closes") + "\n}".len();
    script[start..end].to_string()
}

#[test]
fn deduped_view_mirrors_the_typed_arrays_at_the_top_level() {
    let result = single_branch_fanout(verifier_branch());
    let json = deduped(&result).expect("view");
    let view: serde_json::Value = serde_json::from_str(&json).unwrap();
    let typed = &view["result"];
    for key in MIRRORED_RESULT_KEYS {
        let mirror = view[key]
            .as_array()
            .unwrap_or_else(|| panic!("top-level {key}: {json}"));
        let source = typed[key]
            .as_array()
            .unwrap_or_else(|| panic!("result.{key}: {json}"));
        assert_eq!(mirror.len(), source.len(), "{key} mirrors every entry");
    }
    assert_eq!(view["evidence"], typed["evidence"]);
    assert_eq!(view["artifacts"], typed["artifacts"]);
    for (mirror, source) in view["commands_run"]
        .as_array()
        .unwrap()
        .iter()
        .zip(typed["commands_run"].as_array().unwrap())
    {
        assert_eq!(mirror["command"], source["command"]);
        assert_eq!(mirror["status"], source["status"]);
        assert!(
            mirror.get("output_summary").is_none(),
            "compact projection: {mirror}"
        );
    }
    for (mirror, source) in view["residual_gaps"]
        .as_array()
        .unwrap()
        .iter()
        .zip(typed["residual_gaps"].as_array().unwrap())
    {
        assert_eq!(mirror["id"], source["id"]);
        assert_eq!(mirror["severity"], source["severity"]);
    }
    // Issue-8 stays: each branch result still appears once.
    assert_eq!(json.matches(SENTINEL).count(), 1, "{json}");
    assert!(view["result"]["data"].get("items").is_none());
    assert!(view["result"]["data"].get("outcomes").is_none());
}

#[test]
fn compat_view_carries_no_top_level_mirrors() {
    let result = single_branch_fanout(verifier_branch());
    let view: serde_json::Value =
        serde_json::from_str(&result_view_json(&result).expect("view")).unwrap();
    for key in MIRRORED_RESULT_KEYS {
        assert!(
            view.get(key).is_none(),
            "compat must not grow a top-level {key}"
        );
    }
}

#[test]
fn a_write_result_mirrors_changed_paths_and_commands() {
    let mut result = WorkflowV2Result::accepted("implemented");
    result.files_changed = vec![crate::WorkflowV2FileRecord::new("src/module.ext")];
    result.commands_run = vec![crate::WorkflowV2CommandRecord {
        kind: crate::WorkflowV2CommandKind::Test,
        command: "cargo test -p module".to_string(),
        status: crate::WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "1 passed".to_string(),
    }];
    result.residual_gaps = vec![WorkflowV2ResidualGap {
        id: "gap-1".to_string(),
        description: "left for later".to_string(),
        severity: Some("low".to_string()),
    }];
    let view: serde_json::Value = serde_json::from_str(&deduped(&result).expect("view")).unwrap();
    assert_eq!(view["files_changed"], serde_json::json!(["src/module.ext"]));
    assert_eq!(
        view["commands_run"],
        serde_json::json!([{ "command": "cargo test -p module", "status": "succeeded" }])
    );
    assert_eq!(
        view["residual_gaps"],
        serde_json::json!([{ "id": "gap-1", "severity": "low" }])
    );
    assert_eq!(
        view["result"]["commands_run"][0]["output_summary"],
        "1 passed"
    );
}

/// The size the mirrors add, measured on the live verify envelope: the compact
/// projections keep the copy well under a fifth of the view.
#[test]
fn the_mirrors_add_a_bounded_share_to_the_live_verify_envelope() {
    let result = live_verify_result();
    let json = deduped(&result).expect("view");
    let mut view: serde_json::Value = serde_json::from_str(&json).unwrap();
    let with_mirrors = json.len();
    for key in MIRRORED_RESULT_KEYS {
        view.as_object_mut().unwrap().remove(key);
    }
    let without = serde_json::to_string(&view).unwrap().len();
    let added = with_mirrors - without;
    assert!(
        added * 5 < without,
        "mirrors add {added} bytes to a {without}-byte view (+{}%)",
        added * 100 / without
    );
    assert!(
        json.matches("output_summary").count() > 0,
        "full records stay under result"
    );
}

/// The defect itself: the live script's `isAccepted`, run under node against
/// the live verify envelope in the shape the host now renders, answers true.
#[test]
fn the_live_is_accepted_predicate_accepts_the_live_accepted_verify_envelope() {
    let result = live_verify_result();
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(
        result.commands_run.len(),
        21,
        "the evidence envelope ran 21 commands"
    );
    let envelope = deduped(&result).expect("view");
    let driver = format!(
        "const env = {envelope};\n{snippet}\nconsole.log(JSON.stringify({{ accepted: isAccepted(env), topLevelCommands: (env.commands_run || []).length }}));\n",
        snippet = live_is_accepted_snippet(),
    );
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("is_accepted.mjs");
    std::fs::write(&path, driver).expect("write driver");
    let out = std::process::Command::new("node")
        .arg(&path)
        .output()
        .expect("node must be available");
    assert!(
        out.status.success(),
        "driver failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let verdict: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("driver prints JSON");
    assert_eq!(verdict["topLevelCommands"], 21);
    assert_eq!(verdict["accepted"], true, "{verdict}");
}

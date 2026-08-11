// #163 failure 3: an implementation wave that wrote nothing advanced anyway.
//
// The pure cases fix what counts as a trace; the two driver cases fix what the
// wave does about it, because "the wave did not advance" is a claim about which
// stages ran and only a host that records its calls can witness it.

use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::lifecycle_driver::review_test_host::{RecordingHost, accepted_report};
use crate::v2::lifecycle_driver::{LifecycleDriver, LifecycleEvidence, LifecycleLimits};

const TASK: &str = "TASK-TDL-010";

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["tasks".to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: TASK.to_string(),
            source_path: "tasks/TASK-TDL-010.md".to_string(),
            ..WorkflowV2TaskUniverseTask::default()
        }],
    }
}

fn driver(host: Arc<RecordingHost>) -> LifecycleDriver {
    LifecycleDriver::new(
        host,
        universe(),
        None,
        None,
        Value::Null,
        std::collections::BTreeSet::new(),
        LifecycleLimits {
            max_repair_iterations: 1,
            max_investigation_iterations: 1,
            implementation_wave_max_parallelism: Some(1),
        },
    )
}

fn ready_item() -> Value {
    json!({
        "item_id": "IMPL-TDL-010",
        "canonical_task_ids": [TASK],
        "work_type": "implementation",
        "target_files": ["crates/archon-trading/src/data_store.rs"],
        "acceptance_criteria": ["the registry migrates to v2"],
        "focused_verification": ["cargo test -p archon-trading data_store_tests"],
        "artifact_requirements": [],
    })
}

/// The observed branch: it ran, it reported a command, and it left nothing
/// behind. `has_concrete_evidence` accepts this — `commands_run` alone is
/// enough for it — which is exactly why the question has to be asked here.
fn silent_branch() -> Value {
    json!({
        "item_id": "IMPL-TDL-010",
        "status": "failed",
        "canonical_task_ids": [TASK],
        "summary": "schema repair failed after bounded retries",
        "commands_run": [{ "command": "cargo check", "exit_code": 0 }],
        "files_changed": [],
        "artifacts": [],
        "data": { "patch_landed": false },
    })
}

fn wave(outcomes: Vec<Value>) -> Value {
    json!({ "status": "failed", "outcomes": outcomes })
}

#[test]
fn a_branch_that_ran_a_command_and_wrote_nothing_is_silence() {
    assert!(wave_left_no_trace(&wave(vec![silent_branch()])));
}

#[test]
fn a_wave_with_no_outcomes_at_all_is_silence() {
    assert!(wave_left_no_trace(&wave(Vec::new())));
    assert!(wave_left_no_trace(&json!({ "status": "failed" })));
}

#[test]
fn a_declared_noop_wave_is_a_legitimate_outcome() {
    // The interaction that matters: a wave where every branch verified there
    // was nothing to write and said so is finished work, not silence. Its
    // `files_changed` is empty and its `patch_landed` is false, and it still
    // passes, because the declaration is the trace.
    let declared_noop = json!({
        "item_id": "IMPL-TDL-010",
        "status": "noop",
        "canonical_task_ids": [TASK],
        "summary": "the registry is already at v2",
        "commands_run": [{ "command": "cargo test -p archon-trading", "exit_code": 0 }],
        "files_changed": [],
        "artifacts": [],
        "data": { "idempotent_noop": true, "patch_landed": false },
    });
    assert!(!wave_left_no_trace(&wave(vec![declared_noop])));
}

#[test]
fn every_kind_of_trace_counts_through_every_envelope() {
    for trace in [
        json!({ "files_changed": ["src/lib.rs"] }),
        json!({ "changed_files": ["src/lib.rs"] }),
        json!({ "artifacts": [{ "path": ".archon/registry.json" }] }),
        json!({ "artifact_paths": [".archon/registry.json"] }),
        json!({ "data": { "patch_landed": true } }),
        json!({ "result": { "data": { "patch_landed": true } } }),
        json!({ "result": { "files_changed": ["src/lib.rs"] } }),
        json!({ "idempotent_noop": true }),
    ] {
        assert!(
            !wave_left_no_trace(&wave(vec![trace.clone()])),
            "{trace} is a trace"
        );
    }
}

/// The real write fan-out envelope: a trimmed `outcomes` view that
/// `outcomes_of` prefers, alongside the full branch results under `items`.
/// Reading only the view would call a wave that wrote a file silent.
#[test]
fn the_write_fanout_envelope_is_read_through_both_of_its_views() {
    let fanout = json!({
        "status": "accepted",
        "write_mode": "worktree",
        "outcomes": [{
            "item_id": "IMPL-TDL-010",
            "status": "accepted",
            "canonical_task_ids": [TASK],
            "evidence": [
                { "kind": "implementation", "summary": "declared target changed" },
                { "kind": "file_changed", "path": "src/data_store.rs", "purpose": "declared target edit" },
            ],
        }],
        "items": [{
            "status": "accepted",
            "files_changed": [{ "path": "src/data_store.rs", "purpose": "declared target edit" }],
            "data": { "item_id": "IMPL-TDL-010", "patch_landed": true },
        }],
    });
    assert!(!wave_left_no_trace(&fanout));

    // The same envelope from a wave where nothing landed: the view carries
    // evidence, but none of it says a file changed.
    let silent = json!({
        "status": "failed",
        "write_mode": "worktree",
        "outcomes": [{
            "item_id": "IMPL-TDL-010",
            "status": "failed",
            "evidence": [{ "kind": "command", "command": "cargo check", "status": "succeeded" }],
        }],
        "items": [{
            "status": "failed",
            "files_changed": [],
            "artifacts": [],
            "data": { "item_id": "IMPL-TDL-010", "patch_landed": false },
        }],
    });
    assert!(wave_left_no_trace(&silent));
}

#[test]
fn one_branch_that_wrote_is_enough_for_the_wave() {
    let mut wrote = silent_branch();
    wrote["status"] = json!("accepted");
    wrote["files_changed"] = json!(["crates/archon-trading/src/data_store.rs"]);
    assert!(!wave_left_no_trace(&wave(vec![silent_branch(), wrote])));
}

/// The recorded shape of the real rejection: an empty patch on a branch that
/// declared no no-op. The fixture is the failure mode this gate exists for, so
/// the gate is asserted against it rather than only against hand-written JSON.
#[test]
fn the_recorded_empty_patch_branch_failure_is_silence() {
    let fixture: Value = serde_json::from_str(
        archon_test_support::fixtures::WFC022_EMPTY_PATCH_NO_NOOP_BRANCH_FAILURE,
    )
    .expect("fixture");
    let branch = json!({
        "item_id": fixture["branch_id"],
        "status": fixture["expected_status"],
        "summary": fixture["old_error"],
        "files_changed": [],
        "artifacts": [],
        "data": { "patch_landed": false },
    });
    assert!(wave_left_no_trace(&wave(vec![branch])));
}

#[tokio::test]
async fn a_silent_wave_blocks_instead_of_advancing_to_remediation() {
    let host = RecordingHost::new(Box::new(|method: &str, id: &str| {
        if method == "fanout" {
            return wave(vec![silent_branch()]);
        }
        if method == "finalReport" {
            return accepted_report();
        }
        panic!("unexpected host call: {method} {id}");
    }));
    let driver = driver(host.clone());
    let mut evidence = LifecycleEvidence::default();
    let mut candidate_ids = Vec::new();

    driver
        .run_implementation_wave(&[ready_item()], 1, 1, &mut candidate_ids, &mut evidence)
        .await
        .expect("the wave reports rather than erroring");

    assert!(
        host.call_ids()
            .contains(&"blocked-silent-implementation-wave-1".to_string()),
        "calls: {:?}",
        host.call_ids()
    );
    assert_eq!(
        host.count_starting_with("remediation-"),
        0,
        "a wave that wrote nothing has nothing to remediate; calls: {:?}",
        host.call_ids()
    );
    assert!(
        candidate_ids.is_empty(),
        "a silent wave credits no implementation candidate"
    );
}

#[tokio::test]
async fn a_wave_of_declared_noops_still_completes_normally() {
    let host = RecordingHost::new(Box::new(|method: &str, id: &str| {
        if method == "fanout" {
            return json!({
                "status": "noop",
                "outcomes": [{
                    "item_id": "IMPL-TDL-010",
                    "status": "noop",
                    "canonical_task_ids": [TASK],
                    "evidence": [{ "kind": "inspection", "summary": "already at v2" }],
                    "files_changed": [],
                    "artifacts": [],
                    "data": { "idempotent_noop": true, "patch_landed": false },
                }],
            });
        }
        panic!("unexpected host call: {method} {id}");
    }));
    let driver = driver(host.clone());
    let mut evidence = LifecycleEvidence::default();
    let mut candidate_ids = Vec::new();

    driver
        .run_implementation_wave(&[ready_item()], 1, 1, &mut candidate_ids, &mut evidence)
        .await
        .expect("a declared-noop wave completes");

    assert_eq!(host.call_ids(), vec!["implementation-wave-1".to_string()]);
    assert_eq!(candidate_ids, vec![TASK.to_string()]);
}

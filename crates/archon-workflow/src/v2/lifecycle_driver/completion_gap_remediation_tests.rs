// A wave that completes nothing must try to FIX the gap before it blocks.
//
// The evidence-repair stage is a read-only reducer, so when the gap is real
// work — "the six claimed tests are absent", a missing flush/sync ordering —
// it can only re-read the evidence and confirm the gap is still there. Observed
// live: the run terminated at `blocked-no-completion-1` on defects no agent was
// ever dispatched to fix, with thirteen downstream tasks never attempted.
//
// Which stage ran is a claim about the shape of the loop, so only a host that
// records its calls can witness it: asserting on the returned ids would pass
// just as happily against a loop that never dispatched a writer.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::lifecycle_driver::review_test_host::RecordingHost;
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
        "item_id": "impl-tdl-010",
        "canonical_task_ids": [TASK],
        "work_type": "implementation",
        "target_files": ["crates/archon-trading/src/data_store.rs"],
        "acceptance_criteria": ["the six named registry tests exist and pass"],
        "focused_verification": ["cargo test -p archon-trading registry"],
        "artifact_requirements": [],
    })
}

/// The evidence reducer cannot close a real gap, so the writer must be asked.
#[tokio::test]
async fn a_wave_that_completes_nothing_dispatches_a_write_branch() {
    let host = RecordingHost::new(Box::new(|_method, call_id| {
        if call_id.starts_with("wave-completion-remediation") {
            // The writer fixes the work and reports the task complete.
            return json!({
                "status": "accepted",
                "summary": "added the six named registry tests",
                "data": { "items": [{
                    "item_id": "impl-tdl-010",
                    "canonical_task_ids": [TASK],
                    "status": "accepted",
                    "summary": "tests added and run",
                    "files_changed": [
                        { "path": "crates/archon-trading/tests/registry_schema_v1.rs" }
                    ],
                    "commands_run": [{
                        "kind": "test",
                        "command": "cargo test -p archon-trading registry",
                        "status": "succeeded",
                        "exit_code": 0,
                        "output_summary": "6 passed",
                    }],
                }]},
            });
        }
        // The read-only reducer can only describe the gap.
        json!({
            "status": "needs_review",
            "summary": "the six claimed tests are absent",
            "data": { "items": [] },
        })
    }));
    let driver = driver(host.clone());
    let items = vec![ready_item()];
    let mut evidence = LifecycleEvidence::default();

    let completed = driver
        .repair_wave_completion_evidence(
            &items,
            &[],
            &items,
            &json!({}),
            &std::collections::BTreeSet::new(),
            1,
            &mut evidence,
        )
        .await
        .expect("repair");

    assert!(
        !host
            .ids_starting_with("wave-completion-remediation")
            .is_empty(),
        "a write branch must be dispatched before blocking, got calls: {:?}",
        host.call_ids()
    );
    assert_eq!(
        completed,
        vec![TASK.to_string()],
        "the writer's completion must count, so the wave advances instead of dying"
    );
}

/// Blocking stays available for work that genuinely cannot be completed: when
/// the writer reports it cannot finish, nothing is credited.
#[tokio::test]
async fn a_writer_that_cannot_finish_still_blocks() {
    let host = RecordingHost::new(Box::new(|_method, call_id| {
        if call_id.starts_with("wave-completion-remediation") {
            return json!({
                "status": "blocked",
                "summary": "provider credentials unavailable; cannot complete",
                "data": { "items": [{
                    "item_id": "impl-tdl-010",
                    "canonical_task_ids": [TASK],
                    "status": "blocked",
                    "summary": "hard blocker",
                }]},
            });
        }
        json!({ "status": "needs_review", "data": { "items": [] } })
    }));
    let driver = driver(host.clone());
    let items = vec![ready_item()];
    let mut evidence = LifecycleEvidence::default();

    let completed = driver
        .repair_wave_completion_evidence(
            &items,
            &[],
            &items,
            &json!({}),
            &std::collections::BTreeSet::new(),
            1,
            &mut evidence,
        )
        .await
        .expect("repair");

    assert!(
        completed.is_empty(),
        "a genuine hard blocker must not be credited: {completed:?}"
    );
}

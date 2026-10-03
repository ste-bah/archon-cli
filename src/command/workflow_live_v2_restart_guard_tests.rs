//! Issue-256 (round 2): a restart is never undone by a live session, on any
//! run kind. A run that is not a fixed decomposition owns no generation, so
//! its session's restore and persistence used to skip the run lock and every
//! check. Driven through the host's own restore and persistence paths.

use super::workflow_live_v2_reuse_content_key_tests::{reuse_test_runner, reuse_test_store};
use super::*;
use archon_workflow::v2::restart::invalidate_generated_v2_call;

const CALL: &str = "inspect-one";
const T1: &str = "2026-10-01T10:00:00+00:00";
const T2: &str = "2026-10-01T11:00:00+00:00";

fn record(attempt: u32, at: &str, accepted: bool) -> WorkflowV2CallRecord {
    let call = WorkflowV2HostCall {
        id: CALL.into(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: Default::default(),
    };
    let result = if accepted {
        let mut result = WorkflowV2Result::accepted("inspected the area");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "read the area",
        ));
        result
    } else {
        WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: "paused and produced no result".into(),
            data: serde_json::json!({ "interrupted": "paused" }),
            ..WorkflowV2Result::default()
        }
    };
    let input = if accepted { "in" } else { "drifted" };
    let mut record =
        WorkflowV2CallRecord::new("wf", call, attempt, input.into(), result, Vec::new());
    record.started_at = at.into();
    record.finished_at = at.into();
    record
}

/// A generated (not fixed) run whose call has an accepted attempt 1 in its
/// history and an interrupted attempt 2 in its slot, and a live session
/// opened on it.
fn session() -> (
    tempfile::TempDir,
    WorkflowStore,
    archon_workflow::WorkflowRun,
    WorkflowV2ResultStore,
    WorkflowScriptHost,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, run) = reuse_test_store(&temp);
    archon_workflow::WorkflowBundle::create_for_run(
        &store,
        &run,
        "export default async function workflow(w) {}",
        archon_workflow::WorkflowBundleOrigin::GeneratedHarness,
    )
    .expect("bundle");
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    v2.save_call_record(&record(1, T1, true)).unwrap();
    v2.save_call_record(&record(2, T2, false)).unwrap();
    let host = WorkflowScriptHost {
        scaffold_hash: "fixture".into(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner: reuse_test_runner(&store, &run, &v2, serde_json::Value::Null, None),
        accumulator: Arc::new(tokio::sync::Mutex::new(WorkflowScriptAccumulator::default())),
        tool_host: std::sync::OnceLock::new(),
        tool_budget: Default::default(),
    };
    (temp, store, run, v2, host)
}

fn assert_slot_still_invalidated(v2: &WorkflowV2ResultStore) {
    let slot = v2.load_call_record(CALL).unwrap().expect("slot");
    assert_eq!(
        (slot.attempt, slot.invalidated_by.as_deref()),
        (2, Some(CALL))
    );
    assert!(v2.last_accepted_call_record(CALL, "in").unwrap().is_none());
}

#[tokio::test]
async fn a_live_history_restore_cannot_undo_a_restart_of_a_generated_run() {
    let (_temp, store, run, v2, host) = session();
    let generation = host.fixed_execution_generation().unwrap();
    assert_eq!(generation, None, "not a fixed decomposition");
    let candidate = v2
        .call_record_for_reuse(&record(1, T1, true).call, "in")
        .unwrap()
        .expect("history candidate");
    assert!(candidate.from_history);

    invalidate_generated_v2_call(&store, &run, CALL).unwrap();

    let restored = host.restore_reused_record(&candidate.record, true, generation);
    assert!(
        matches!(restored, Err(WorkflowError::ControlCancelled(_))),
        "{restored:?}"
    );
    assert_slot_still_invalidated(&v2);
}

#[tokio::test]
async fn a_live_result_cannot_be_persisted_over_a_restart_of_a_generated_run() {
    let (_temp, store, run, v2, host) = session();

    invalidate_generated_v2_call(&store, &run, CALL).unwrap();

    let persisted = host
        .persist_generation_owned_call_and_emit(
            &record(3, T2, true),
            crate::command::workflow_decompose_state::FixedCallProjectionKind::Executed,
            None,
        )
        .await;
    assert!(
        matches!(persisted, Err(WorkflowError::ControlCancelled(_))),
        "{persisted:?}"
    );
    assert_slot_still_invalidated(&v2);
}

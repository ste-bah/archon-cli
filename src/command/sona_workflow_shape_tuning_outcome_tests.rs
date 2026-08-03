use super::*;
use archon_workflow::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result, WorkflowV2Status,
};

fn call_record(call_id: &str, outcomes: serde_json::Value) -> WorkflowV2CallRecord {
    let mut result = WorkflowV2Result::accepted("seeded");
    result.data = serde_json::json!({ "outcomes": outcomes });
    WorkflowV2CallRecord {
        run_id: "run".to_string(),
        call: WorkflowV2HostCall {
            id: call_id.to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        attempt: 1,
        schema_version: "workflow-v2-call-record-v1".to_string(),
        started_at: String::new(),
        finished_at: String::new(),
        input_hash: String::new(),
        output_hash: String::new(),
        status: WorkflowV2Status::Accepted,
        result,
        depends_on: Vec::new(),
        invalidated_by: None,
        agent_session_id: None,
        source_fingerprint: None,
        source_task_graph: None,
        completed_ids: Vec::new(),
        scaffold_hash: None,
        completion_evidence: Vec::new(),
        evidence_snapshot_hash: None,
    }
}

fn seed_run(records: &[WorkflowV2CallRecord]) -> (tempfile::TempDir, WorkflowStore, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    let spec = archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "shape-observation".to_string(),
        task: "observe fan-out contention".to_string(),
        target_repository_root: None,
        max_parallelism: 4,
        max_agents: 16,
        stages: Vec::new(),
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    };
    let run = store.create_run(spec).expect("create run");
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    for record in records {
        v2.save_call_record(record).expect("save call record");
    }
    let run_id = run.id.clone();
    (temp, store, run_id)
}

fn observe(records: &[WorkflowV2CallRecord]) -> Vec<TuningObservation> {
    let (_temp, store, run_id) = seed_run(records);
    observe_run(&store, &run_id)
}

const CLEAN: fn() -> serde_json::Value = || serde_json::json!([{ "status": "accepted" }]);

/// A run with no write fan-out says nothing about how wide one should be.
/// Inventing a neutral row for it would inflate the count that gates the loop.
#[test]
fn a_run_with_no_write_wave_produces_no_observation() {
    let observations = observe(&[
        call_record("verification-wave-1", CLEAN()),
        call_record("adversarial-review-1", CLEAN()),
        call_record("cross-cutting-review-1", CLEAN()),
    ]);
    assert!(observations.is_empty());
}

#[test]
fn an_unreadable_run_produces_no_observation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    assert!(observe_run(&store, "wf-does-not-exist").is_empty());
}

/// The ratchet: a clean write wave is evidence that the fan-out was observed
/// and no evidence at all about whether it should be wider.
#[test]
fn a_clean_write_wave_records_the_neutral_pressure() {
    let observations = observe(&[call_record("implementation-wave-1", CLEAN())]);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].pressure, NEUTRAL_PRESSURE);
    assert_eq!(
        observations[0].parameter_key,
        TunableShapeKnob::ImplementationWaveFanoutWidth.key()
    );
}

#[test]
fn an_observed_worktree_lock_records_full_contention_pressure() {
    let observations = observe(&[call_record(
        "implementation-wave-2",
        serde_json::json!([
            { "status": "accepted" },
            { "error": "branch failed: worktree lock held by another branch" },
        ]),
    )]);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].pressure, 1.0);
}

#[test]
fn every_write_wave_family_is_observed() {
    for family in [
        "implementation-wave-1",
        "remediation-wave-1",
        "review-remediation-wave-1",
    ] {
        assert!(
            is_write_wave_call(family),
            "{family} holds a worktree and must be observed"
        );
        let observations = observe(&[call_record(
            family,
            serde_json::json!([{ "error": "git index.lock exists" }]),
        )]);
        assert_eq!(observations.len(), 1, "{family}");
        assert_eq!(observations[0].pressure, 1.0, "{family}");
    }
}

/// Read-only stages share no writable state. Attributing their failures to the
/// write fan-out's width would narrow a wave that was never the constraint.
#[test]
fn read_only_stages_are_not_write_waves() {
    for family in [
        "verification-wave-1",
        "review-verification-wave-1",
        "noop-proof-verification-1",
        "adversarial-review-1",
        "artifact-existence-investigation-1",
    ] {
        assert!(!is_write_wave_call(family), "{family} holds no worktree");
    }
}

/// Contention in a read-only stage must not reach the width knob at all.
#[test]
fn contention_in_a_read_only_stage_does_not_narrow_the_fan_out() {
    let observations = observe(&[
        call_record("implementation-wave-1", CLEAN()),
        call_record(
            "verification-wave-1",
            serde_json::json!([{ "error": "worktree lock held" }]),
        ),
    ]);
    assert_eq!(observations.len(), 1);
    assert_eq!(
        observations[0].pressure, NEUTRAL_PRESSURE,
        "a read-only stage's contention is not evidence about write-wave width"
    );
}

/// The matcher is deliberately narrow: a false positive narrows a wave that was
/// fine, and the errors below are ordinary failures with no concurrency in them.
#[test]
fn ordinary_branch_failures_are_not_read_as_contention() {
    for error in [
        Some("branch failed: tests did not pass"),
        Some("host call timed out after 7200s"),
        Some("agent returned malformed JSON"),
        Some("file not found"),
        None,
    ] {
        assert!(
            !is_contention_error(error),
            "{error:?} is not concurrency contention"
        );
    }
}

#[test]
fn contention_wordings_are_matched_case_insensitively() {
    for error in [
        "Worktree Lock could not be acquired",
        "WRITE CONFLICT detected between branches",
        "the worktree is locked by pid 4242",
        "another branch already locked this path",
    ] {
        assert!(is_contention_error(Some(error)), "{error}");
    }
}

/// Recording must never alter a result, so every fail-closed exit is silent.
#[test]
fn recording_without_sona_consent_writes_nothing() {
    let (temp, store, run_id) = seed_run(&[call_record("implementation-wave-1", CLEAN())]);
    let mut learning = LearningConfig::default();
    learning.sona.enabled = true;
    learning.sona.pipeline_recording = false;

    record_generated_shape_outcome(temp.path(), &store, &run_id, "bug-hunt", &learning);

    assert!(
        !learning_store_path(temp.path()).exists(),
        "a project that never consented must not gain a learning store"
    );
}

#[test]
fn recording_a_run_with_no_write_wave_writes_nothing() {
    let (temp, store, run_id) = seed_run(&[call_record("verification-wave-1", CLEAN())]);
    let mut learning = LearningConfig::default();
    learning.sona.enabled = true;
    learning.sona.pipeline_recording = true;

    record_generated_shape_outcome(temp.path(), &store, &run_id, "bug-hunt", &learning);

    assert!(
        !learning_store_path(temp.path()).exists(),
        "no observation means no store"
    );
}

/// The round trip: a recorded contention observation is readable by the read
/// half on the same route, which is what makes the fail-closed rule checkable —
/// delete the rows and the weight is gone, not merely hidden.
#[test]
fn a_recorded_observation_is_readable_by_the_read_half() {
    let (temp, store, run_id) = seed_run(&[call_record(
        "implementation-wave-1",
        serde_json::json!([{ "error": "worktree lock contention" }]),
    )]);
    let mut learning = LearningConfig::default();
    learning.sona.enabled = true;
    learning.sona.pipeline_recording = true;

    record_generated_shape_outcome(temp.path(), &store, &run_id, "bug-hunt", &learning);

    let db =
        crate::command::topology_fold::open_store(&learning_store_path(temp.path()), "learning")
            .expect("store exists after recording");
    let read_back = load_shape_observations(&db, "bug-hunt");
    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back[0].pressure, 1.0);
    assert_eq!(
        read_back[0].parameter_key,
        TunableShapeKnob::ImplementationWaveFanoutWidth.key()
    );
    assert!(
        load_shape_observations(&db, "greenfield").is_empty(),
        "another class must not see this run's evidence"
    );
}

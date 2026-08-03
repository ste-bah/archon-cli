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

fn seed_run(
    records: &[WorkflowV2CallRecord],
    completed: bool,
) -> (tempfile::TempDir, WorkflowStore, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    let spec = archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "tuning-observation".to_string(),
        task: "observe budget pressure".to_string(),
        target_repository_root: None,
        max_parallelism: 4,
        max_agents: 16,
        stages: Vec::new(),
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    };
    let mut run = store.create_run(spec).expect("create run");
    if completed {
        run.status = archon_workflow::RunStatus::Completed;
        store.save_state(&run).expect("save state");
    }
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    for record in records {
        v2.save_call_record(record).expect("save call record");
    }
    let run_id = run.id.clone();
    (temp, store, run_id)
}

fn pressure(
    observations: &[TuningObservation],
    parameter: TunableGeneratedParameter,
) -> Option<f64> {
    observations
        .iter()
        .find(|observation| observation.parameter_key == parameter.key())
        .map(|observation| observation.pressure)
}

/// The ratchet, stated directly: a clean run records evidence but no downward
/// pressure, so no run of successes can shorten a verification timeout. This is
/// the property that makes the observed 1200s incident unreachable through
/// learning rather than merely unlikely.
#[test]
fn a_clean_run_never_records_downward_pressure_on_a_timeout() {
    let (_temp, store, run_id) = seed_run(
        &[call_record(
            "verification-1",
            serde_json::json!([{"error": null}]),
        )],
        true,
    );

    let observations = observe_run(&store, &run_id);

    assert_eq!(
        pressure(
            &observations,
            TunableGeneratedParameter::VerificationBranchTimeoutSecs
        ),
        Some(NEUTRAL_PRESSURE)
    );
    assert_eq!(
        pressure(
            &observations,
            TunableGeneratedParameter::HostCallTimeoutSecs
        ),
        Some(NEUTRAL_PRESSURE)
    );
}

/// An observed verification timeout is attributed to the verification budget
/// and not to the host-call budget.
#[test]
fn a_verification_timeout_pressures_only_the_verification_budget() {
    let (_temp, store, run_id) = seed_run(
        &[
            call_record(
                "verification-wave-1",
                serde_json::json!([{"error": "branch timed out after 1200s"}]),
            ),
            call_record(
                "implementation-wave-1",
                serde_json::json!([{"error": null}]),
            ),
        ],
        false,
    );

    let observations = observe_run(&store, &run_id);

    assert_eq!(
        pressure(
            &observations,
            TunableGeneratedParameter::VerificationBranchTimeoutSecs
        ),
        Some(1.0)
    );
    assert_eq!(
        pressure(
            &observations,
            TunableGeneratedParameter::HostCallTimeoutSecs
        ),
        Some(NEUTRAL_PRESSURE)
    );
}

/// A coder branch timing out pressures the host-call budget instead.
#[test]
fn an_implementation_timeout_pressures_only_the_host_call_budget() {
    let (_temp, store, run_id) = seed_run(
        &[call_record(
            "implementation-wave-1",
            serde_json::json!([{"error": "host call timeout"}]),
        )],
        false,
    );

    let observations = observe_run(&store, &run_id);

    assert_eq!(
        pressure(
            &observations,
            TunableGeneratedParameter::HostCallTimeoutSecs
        ),
        Some(1.0)
    );
    assert_eq!(
        pressure(
            &observations,
            TunableGeneratedParameter::VerificationBranchTimeoutSecs
        ),
        Some(NEUTRAL_PRESSURE)
    );
}

/// A run that resolved without entering the repair loop is real evidence that
/// the cap was never needed.
#[test]
fn a_resolved_run_that_never_repaired_records_zero_iteration_pressure() {
    let (_temp, store, run_id) = seed_run(
        &[call_record("implementation-wave-1", serde_json::json!([]))],
        true,
    );

    let observations = observe_run(&store, &run_id);

    assert_eq!(
        pressure(
            &observations,
            TunableGeneratedParameter::MaxRepairIterations
        ),
        Some(0.0)
    );
}

/// A run that entered the repair loop and still did not resolve is the case the
/// budget is the suspect for.
#[test]
fn an_unresolved_run_that_repaired_records_full_iteration_pressure() {
    let (_temp, store, run_id) = seed_run(
        &[call_record("remediation-wave-1", serde_json::json!([]))],
        false,
    );

    let observations = observe_run(&store, &run_id);

    assert_eq!(
        pressure(
            &observations,
            TunableGeneratedParameter::MaxRepairIterations
        ),
        Some(1.0)
    );
}

/// A run that failed without ever entering the loop records nothing for that
/// budget: attributing an unrelated failure to it would both move the weight
/// and inflate the evidence count that gates the whole loop.
#[test]
fn an_unresolved_run_that_never_repaired_records_no_iteration_evidence() {
    let (_temp, store, run_id) = seed_run(
        &[call_record("implementation-wave-1", serde_json::json!([]))],
        false,
    );

    let observations = observe_run(&store, &run_id);

    assert_eq!(
        pressure(
            &observations,
            TunableGeneratedParameter::MaxRepairIterations
        ),
        None
    );
    assert_eq!(
        pressure(
            &observations,
            TunableGeneratedParameter::MaxInvestigationIterations
        ),
        None
    );
}

/// A run that left no persisted calls left no evidence.
#[test]
fn a_run_with_no_persisted_calls_produces_no_observations() {
    let (_temp, store, run_id) = seed_run(&[], true);

    assert!(observe_run(&store, &run_id).is_empty());
}

#[test]
fn both_timeout_spellings_the_runtime_emits_are_recognised() {
    assert!(is_timeout_error(Some("branch timed out after 1200s")));
    assert!(is_timeout_error(Some("Host call TIMEOUT")));
    assert!(!is_timeout_error(Some("compilation failed")));
    assert!(!is_timeout_error(None));
}

/// The consent gate is the same one that lets SONA record at all.
#[test]
fn recording_is_a_no_op_without_pipeline_recording_consent() {
    let mut learning = LearningConfig::default();
    learning.sona.enabled = true;
    learning.sona.pipeline_recording = false;
    assert!(!sona_tuning_enabled(&learning));

    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    record_generated_tuning_outcome(temp.path(), &store, "missing-run", "review", &learning);

    assert!(
        !learning_store_path(temp.path()).exists(),
        "a disabled tuner must not create a learning store"
    );
}

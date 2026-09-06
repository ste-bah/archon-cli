use super::workflow_live_v2_finalizer::{
    FINALIZATION_RECORD_PATH, WorkflowRunEndObserver, finalize_summary,
};
use super::workflow_live_v2_script::WorkflowV2ScriptSummary;
use super::workflow_run_end_observer::{
    FixedRunEndAcceptanceObserver, RUN_END_OBSERVER_RECORDS_PATH,
};
use super::*;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceCheck, AcceptanceContract,
    AcceptanceCriterion, AcceptanceLock, AcceptancePin, FreezeGateMode, FreezeGateStamp, GapPolicy,
    JudgeDecision, JudgeVerdict, PrdIdentity, ResidualGapRecord, TrustedCwd, content_digest,
    empty_gate_findings_digest,
};
use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton, TaskSkeletonLock};
use archon_workflow::task_universe::WorkflowV2DeliverableContract;
use archon_workflow::{
    ObserverAuthority, PortableAcceptanceIdentityV1, RunEndAcceptanceObserverSnapshotV1,
    RunEndObserverStateV1, WorkflowEvent, WorkflowRunKind, WorkflowSpec,
};
use std::collections::BTreeSet;
fn stamp() -> FreezeGateStamp {
    FreezeGateStamp {
        mode: FreezeGateMode::Observe,
        finding_count: 0,
        findings_digest: empty_gate_findings_digest(),
        binary_commit: "test-revision".into(),
        evaluated_at: "2026-08-27T00:00:00Z".into(),
    }
}
fn floor(path: &str) -> AcceptanceCheck {
    AcceptanceCheck::Floor {
        contract: WorkflowV2DeliverableContract {
            kind: "proof".into(),
            artifact_path: path.into(),
            ..WorkflowV2DeliverableContract::default()
        },
    }
}

fn command(text: String) -> AcceptanceCheck {
    AcceptanceCheck::Command {
        command: text,
        cwd: TrustedCwd::ProjectRoot,
    }
}
fn criterion(id: &str, check: AcceptanceCheck) -> AcceptanceCriterion {
    AcceptanceCriterion {
        id: id.into(),
        criterion: format!("criterion {id}"),
        check,
        gap_permitted: false,
        judgment: JudgeVerdict {
            verdict: JudgeDecision::Accepted,
            counterexample: "missing output".into(),
            reason: "the declared check rejects it".into(),
            sampling: None,
            host_call_id: "judge-1".into(),
        },
    }
}
struct FrozenFixture {
    project: tempfile::TempDir,
    store: WorkflowStore,
    task_root: std::path::PathBuf,
    snapshot: RunEndAcceptanceObserverSnapshotV1,
}

fn frozen_fixture(checks: Vec<AcceptanceCriterion>) -> FrozenFixture {
    frozen_fixture_with_permitted(checks, BTreeSet::new())
}

fn frozen_fixture_with_permitted(
    mut checks: Vec<AcceptanceCriterion>,
    permitted: BTreeSet<String>,
) -> FrozenFixture {
    for criterion in &mut checks {
        criterion.gap_permitted = permitted.contains(&criterion.id);
    }
    let project = tempfile::tempdir().unwrap();
    let task_root = project.path().join("tasks/set");
    std::fs::create_dir_all(&task_root).unwrap();
    let contract = AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "prds/PRD-X.md".into(),
            digest: "prd-digest".into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: permitted,
            forbidden_phrases: vec!["later".into()],
            required_fields: Vec::new(),
        },
        acceptance: checks,
        supplementary: Vec::new(),
    };
    let acceptance_bytes = serde_json::to_vec_pretty(&contract).unwrap();
    let acceptance_digest = content_digest(&acceptance_bytes);
    std::fs::write(task_root.join(ACCEPTANCE_CONTRACT_FILE), acceptance_bytes).unwrap();
    std::fs::write(
        task_root.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&AcceptanceLock {
            algorithm: "blake3".into(),
            digest: acceptance_digest.clone(),
            gate: stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: acceptance_digest.clone(),
        tasks: vec![FrozenTask {
            task_id: "TASK-EX-001".into(),
            file_name: "TASK-EX-001.md".into(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: contract
                .acceptance
                .iter()
                .map(|item| item.id.clone())
                .collect(),
            deliverable_contracts: Vec::new(),
        }],
    };
    let skeleton_bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    let skeleton_digest = content_digest(&skeleton_bytes);
    std::fs::write(
        task_root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
        skeleton_bytes,
    )
    .unwrap();
    std::fs::write(
        task_root.join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: skeleton_digest.clone(),
            acceptance_digest: acceptance_digest.clone(),
            gate: stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let pin = AcceptancePin {
        task_root: task_root.canonicalize().unwrap().display().to_string(),
        acceptance_digest: acceptance_digest.clone(),
        freeze_event_id: "freeze-observer".into(),
        acceptance_gate: stamp(),
        skeleton_digest: Some(skeleton_digest.clone()),
        skeleton_gate: Some(stamp()),
    };
    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(project.path(), &task_root);
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    std::fs::write(pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    let snapshot = RunEndAcceptanceObserverSnapshotV1 {
        native_execution: None,
        schema_version: 1,
        canonical_task_root_identity: task_root.canonicalize().unwrap().display().to_string(),
        expected_artifact_paths: [
            ACCEPTANCE_CONTRACT_FILE,
            ACCEPTANCE_LOCK_FILE,
            archon_workflow::task_set_contract::TASK_SKELETON_FILE,
            archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE,
            "acceptance-pin.json",
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        portable_acceptance_identity: Some(PortableAcceptanceIdentityV1 {
            freeze_event_id: pin.freeze_event_id,
            acceptance_digest,
            skeleton_digest: Some(skeleton_digest),
        }),
    };
    let store = WorkflowStore::project(project.path());
    FrozenFixture {
        project,
        store,
        task_root,
        snapshot,
    }
}

fn context<'a>(
    fixture: &'a FrozenFixture,
    run_id: &'a str,
) -> super::workflow_live_v2_finalizer::RunEndObserverContext<'a> {
    super::workflow_live_v2_finalizer::RunEndObserverContext {
        run_id,
        terminal_status: WorkflowV2Status::Accepted,
        snapshot: &fixture.snapshot,
    }
}

fn records(fixture: &FrozenFixture, run_id: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(
        fixture
            .store
            .run_dir(run_id)
            .join(RUN_END_OBSERVER_RECORDS_PATH),
    )
    .unwrap()
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect()
}

#[test]
fn failed_commandless_floor_writes_observe_only_shadow() {
    let fixture = frozen_fixture(vec![criterion("AC-X-001", floor("missing.json"))]);
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());
    let outcome = observer.observe(&context(&fixture, &run.id)).unwrap();

    assert_eq!(outcome.authority, ObserverAuthority::ObserveOnly);
    assert_eq!(outcome.evaluated_floor_count, 1);
    assert_eq!(outcome.policy_finding_count, 1);
    assert_eq!(outcome.operational_deferral_count, 0);
    let rows = records(&fixture, &run.id);
    assert_eq!(rows[0]["record_kind"], "policy_shadow");
    assert_eq!(rows[0]["acceptance_id"], "AC-X-001");
    assert_eq!(rows[0]["authority"], "observe_only");
}

#[test]
fn command_check_is_deferred_and_never_executed() {
    let project = tempfile::tempdir().unwrap();
    let sentinel = project.path().join("must-not-exist");
    let fixture = frozen_fixture(vec![criterion(
        "AC-X-001",
        command(format!("touch {}", sentinel.display())),
    )]);
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());

    let outcome = observer.observe(&context(&fixture, &run.id)).unwrap();

    assert_eq!(outcome.evaluated_floor_count, 0);
    assert_eq!(outcome.operational_deferral_count, 1);
    assert!(!sentinel.exists());
    let encoded = serde_json::to_string(&records(&fixture, &run.id)).unwrap();
    assert!(!encoded.contains("touch"));
    assert!(!encoded.contains("must-not-exist"));
}

fn residual(acceptance_id: &str) -> ResidualGapRecord {
    ResidualGapRecord {
        id: format!("GAP-{acceptance_id}"),
        acceptance_id: acceptance_id.into(),
        area: "runtime".into(),
        description: "concrete unmet condition".into(),
        impact: "acceptance remains unproved".into(),
        fail_closed_behavior: "promotion remains disabled".into(),
        owner: "runtime owner".into(),
        created_at: "2026-08-27T00:00:00Z".into(),
        fail_closed_check: "jq -e '.ready == true' state.json".into(),
    }
}

#[test]
fn failed_floor_with_valid_residual_records_shadow_and_command_deferral() {
    let permitted = BTreeSet::from(["AC-X-001".to_string()]);
    let fixture = frozen_fixture_with_permitted(
        vec![criterion("AC-X-001", floor("missing.json"))],
        permitted,
    );
    std::fs::write(
        fixture
            .task_root
            .join(archon_workflow::task_set_contract::RESIDUAL_GAPS_FILE),
        serde_json::to_vec_pretty(&vec![residual("AC-X-001")]).unwrap(),
    )
    .unwrap();
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());

    let outcome = observer.observe(&context(&fixture, &run.id)).unwrap();

    assert_eq!(outcome.policy_finding_count, 1);
    assert_eq!(outcome.operational_deferral_count, 1);
    let rows = records(&fixture, &run.id);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| row["record_kind"] == "policy_shadow"));
    assert!(
        rows.iter()
            .any(|row| row["record_kind"] == "operational_deferral")
    );
    let encoded = serde_json::to_string(&rows).unwrap();
    assert!(!encoded.contains("jq -e"));
}

#[test]
fn residual_for_passing_floor_is_stale_and_leaves_no_partial_records() {
    let permitted = BTreeSet::from(["AC-X-001".to_string()]);
    let fixture =
        frozen_fixture_with_permitted(vec![criterion("AC-X-001", floor("proof.json"))], permitted);
    std::fs::write(
        fixture.project.path().join("proof.json"),
        r#"{"ready":true}"#,
    )
    .unwrap();
    std::fs::write(
        fixture
            .task_root
            .join(archon_workflow::task_set_contract::RESIDUAL_GAPS_FILE),
        serde_json::to_vec_pretty(&vec![residual("AC-X-001")]).unwrap(),
    )
    .unwrap();
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());

    let error = observer.observe(&context(&fixture, &run.id)).unwrap_err();

    assert!(error.to_string().contains("stale"), "{error}");
    assert!(
        !fixture
            .store
            .run_dir(&run.id)
            .join(RUN_END_OBSERVER_RECORDS_PATH)
            .exists()
    );
}

#[test]
fn global_enforce_mode_cannot_promote_observer_authority() {
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.gate_mode = archon_core::config::GateMode::Enforce;
    assert_eq!(
        config.workflow.gate_mode,
        archon_core::config::GateMode::Enforce
    );
    let fixture = frozen_fixture(vec![criterion("AC-X-001", floor("missing.json"))]);
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());

    let outcome = observer.observe(&context(&fixture, &run.id)).unwrap();

    assert_eq!(outcome.authority, ObserverAuthority::ObserveOnly);
    assert_eq!(outcome.policy_finding_count, 1);
}

#[test]
fn replaced_expected_chain_is_operational_failure_not_silence() {
    let fixture = frozen_fixture(vec![criterion("AC-X-001", floor("missing.json"))]);
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    std::fs::write(
        fixture.task_root.join(ACCEPTANCE_CONTRACT_FILE),
        b"replaced",
    )
    .unwrap();
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());

    let error = observer.observe(&context(&fixture, &run.id)).unwrap_err();

    assert!(error.to_string().contains("acceptance"), "{error}");
    assert!(
        !fixture
            .store
            .run_dir(&run.id)
            .join(RUN_END_OBSERVER_RECORDS_PATH)
            .exists()
    );
}

#[tokio::test]
async fn finalizer_commits_terminal_event_before_real_observer_shadow() {
    let fixture = frozen_fixture(vec![criterion("AC-X-001", floor("missing.json"))]);
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(fixture.store.run_dir(&run.id).join("v2"));
    seed_finalizer_call(&v2_store);
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());

    finalize_summary(
        &fixture.store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        Some(fixture.snapshot.clone()),
        &finalizer_summary(),
        &v2_store,
        Some(&observer),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        fixture.store.load_state(&run.id).unwrap().status,
        RunStatus::Completed
    );
    let events = read_events(&fixture.store, &run.id);
    let terminal = events
        .iter()
        .position(|event| event.detail["event"] == "terminal_status")
        .unwrap();
    let shadow = events
        .iter()
        .position(|event| event.detail["event"] == "run_end_acceptance_shadow_observed")
        .unwrap();
    assert!(terminal < shadow);
    let finalization: archon_workflow::FinalizationRecordV1 = serde_json::from_slice(
        &std::fs::read(
            fixture
                .store
                .run_dir(&run.id)
                .join(FINALIZATION_RECORD_PATH),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        finalization.observer_state,
        Some(RunEndObserverStateV1::Completed { .. })
    ));
}

fn finalizer_spec() -> WorkflowSpec {
    WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "observer-test".into(),
        task: "observe".into(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: vec![archon_workflow::StageSpec {
            id: "call-1".into(),
            kind: archon_workflow::StageKind::Agent,
            task: Some("test".into()),
            agent: None,
            foreach: None,
            reducer: None,
            tool: None,
            depends_on: Vec::new(),
            provider_tier: None,
            retry: Default::default(),
            input: serde_json::Value::Null,
            model: None,
            provider: None,
            expected_target_files: Vec::new(),
            verify_command: None,
            max_parallelism: None,
            item_kind: None,
            filter: None,
            extra: Default::default(),
        }],
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    }
}

fn finalizer_summary() -> WorkflowV2ScriptSummary {
    WorkflowV2ScriptSummary {
        status: WorkflowV2Status::Accepted,
        completed: 1,
        executed: 1,
        reused: 0,
        calls: vec![WorkflowV2HostCall {
            id: "call-1".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        }],
        failed_call: None,
        failed_result_path: None,
        next_action: None,
        script_result: None,
    }
}

fn seed_finalizer_call(store: &WorkflowV2ResultStore) {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "done".into(),
        ..Default::default()
    };
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            store.run_id(),
            finalizer_summary().calls[0].clone(),
            1,
            "input".into(),
            result,
            Vec::new(),
        ))
        .unwrap();
}

fn read_events(store: &WorkflowStore, run_id: &str) -> Vec<WorkflowEvent> {
    std::fs::read_to_string(store.events_path(run_id))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[path = "workflow_run_end_native_tests.rs"]
mod native_tests;

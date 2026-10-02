//! The ACC-A9 heal tests' fixture: a frozen one-check task set, a git repo,
//! and a run whose in-run acceptance round passed.

use super::*;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceCheck, AcceptanceContract,
    AcceptanceCriterion, AcceptanceLock, AcceptancePin, FreezeGateMode, FreezeGateStamp, GapPolicy,
    JudgeDecision, JudgeVerdict, PrdIdentity, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
    TrustedCwd, content_digest, empty_gate_findings_digest,
};
use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton, TaskSkeletonLock};
use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;
use archon_workflow::{
    WorkflowSpec, WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2HostCall,
    WorkflowV2HostMethod,
};

fn stamp() -> FreezeGateStamp {
    FreezeGateStamp {
        mode: FreezeGateMode::Observe,
        finding_count: 0,
        findings_digest: empty_gate_findings_digest(),
        binary_commit: "test-revision".into(),
        evaluated_at: "2026-09-30T00:00:00Z".into(),
    }
}

pub(super) struct Fixture {
    pub(super) _project: tempfile::TempDir,
    pub(super) repo: tempfile::TempDir,
    pub(super) task_root: std::path::PathBuf,
    pub(super) store: WorkflowStore,
    pub(super) v2_store: WorkflowV2ResultStore,
    pub(super) runtime: WorkflowV2ScriptRuntime,
    pub(super) universe: WorkflowV2TaskUniverse,
    pub(super) run_id: String,
}

fn write<T: serde::Serialize>(path: std::path::PathBuf, value: &T) -> Vec<u8> {
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    std::fs::write(path, &bytes).unwrap();
    bytes
}

/// A frozen one-check task set (REQ-1: `present` exists) over a git repo
/// that has it, and a run whose in-run acceptance round passed.
pub(super) fn fixture() -> Fixture {
    let project = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "user.name", "fixture"]);
    std::fs::write(repo.path().join("present"), "x").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "fixture"]);
    let task_root = project.path().join("tasks/set");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::write(project.path().join("prd.md"), "# PRD\n").unwrap();
    std::fs::write(task_root.join("TASK-H-001.md"), "# TASK-H-001\n").unwrap();
    let contract = AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "prd.md".into(),
            digest: "prd-digest".into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: Default::default(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: vec![AcceptanceCriterion {
            id: "REQ-1".into(),
            criterion: "the deliverable exists".into(),
            check: AcceptanceCheck::Command {
                command: "test -f present".into(),
                cwd: TrustedCwd::RepoRoot,
            },
            gap_permitted: false,
            covers: Vec::new(),
            judgment: JudgeVerdict {
                verdict: JudgeDecision::Accepted,
                counterexample: "missing deliverable".into(),
                reason: "the declared check rejects it".into(),
                sampling: None,
                host_call_id: "judge-1".into(),
            },
        }],
        supplementary: Vec::new(),
    };
    let acceptance_digest =
        content_digest(&write(task_root.join(ACCEPTANCE_CONTRACT_FILE), &contract));
    let lock = AcceptanceLock {
        algorithm: "blake3".into(),
        digest: acceptance_digest.clone(),
        gate: stamp(),
    };
    write(task_root.join(ACCEPTANCE_LOCK_FILE), &lock);
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: acceptance_digest.clone(),
        tasks: vec![FrozenTask {
            task_id: "TASK-H-001".into(),
            file_name: "TASK-H-001.md".into(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: vec!["REQ-1".into()],
            deliverable_contracts: Vec::new(),
        }],
    };
    let skeleton_digest = content_digest(&write(task_root.join(TASK_SKELETON_FILE), &skeleton));
    let skeleton_lock = TaskSkeletonLock {
        algorithm: "blake3".into(),
        digest: skeleton_digest.clone(),
        acceptance_digest: acceptance_digest.clone(),
        gate: stamp(),
    };
    write(task_root.join(TASK_SKELETON_LOCK_FILE), &skeleton_lock);
    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(project.path(), &task_root);
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    let pin = AcceptancePin {
        check_sources_digest: None,
        task_root: task_root
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap()
            .display()
            .to_string(),
        acceptance_digest,
        freeze_event_id: "freeze-fixture".into(),
        acceptance_gate: stamp(),
        skeleton_digest: Some(skeleton_digest),
        skeleton_gate: Some(stamp()),
        fidelity_waivers: Vec::new(),
        lineage: Vec::new(),
        lineage_recording: None,
    };
    write(pin_path, &pin);
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![task_root.display().to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-H-001".into(),
            source_path: task_root.join("TASK-H-001.md").display().to_string(),
            implements: vec!["REQ-1".into()],
            ..WorkflowV2TaskUniverseTask::default()
        }],
    };
    let store = WorkflowStore::project(project.path());
    let run = store
        .create_run(WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "run-end-heal".into(),
            task: "run-end heal".into(),
            target_repository_root: Some(repo.path().display().to_string()),
            max_parallelism: 1,
            max_agents: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    Fixture {
        _project: project,
        runtime: WorkflowV2ScriptRuntime {
            target_repository_root: Some(repo.path().display().to_string()),
            generated_config: Default::default(),
        },
        repo,
        task_root,
        store,
        v2_store,
        universe,
        run_id: run.id,
    }
}

fn acceptance_call() -> WorkflowV2HostCall {
    let (options, _) = archon_workflow::v2::script::parse_script_options(&serde_json::json!({
        "tool": archon_workflow::v2::acceptance_stage::ACCEPTANCE_STAGE_TOOL,
        "round": 1,
        "maxRounds": 3,
        "checkIds": [],
    }))
    .unwrap();
    WorkflowV2HostCall {
        id: "acceptance-contract-run-1".into(),
        method: WorkflowV2HostMethod::Tool,
        write_mode: None,
        options,
    }
}

/// An in-run round: run the stage and record its call as the script host
/// would, so the gate binds to it.
pub(super) async fn in_run_round(fixture: &Fixture) -> WorkflowV2ScriptSummary {
    let execution = WorkflowV2CallExecution {
        call: acceptance_call(),
        input: serde_json::json!({}),
        depends_on: Vec::new(),
    };
    let result = super::super::super::workflow_live_v3_acceptance::run_acceptance_stage(
        &fixture.runtime,
        &execution,
        &fixture.store,
        &fixture.run_id,
        Some(&fixture.universe),
        None,
    )
    .await
    .expect("in-run round");
    let record = WorkflowV2CallRecord::new(
        fixture.v2_store.run_id(),
        acceptance_call(),
        1,
        "input".into(),
        result,
        Vec::new(),
    );
    fixture.v2_store.save_call_record(&record).unwrap();
    WorkflowV2ScriptSummary {
        status: WorkflowV2Status::Accepted,
        completed: 1,
        executed: 1,
        reused: 0,
        calls: vec![acceptance_call()],
        failed_call: None,
        failed_result_path: None,
        next_action: None,
        script_result: None,
    }
}

pub(super) fn snapshot(fixture: &Fixture) -> RunEndAcceptanceObserverSnapshotV1 {
    RunEndAcceptanceObserverSnapshotV1 {
        native_execution: None,
        schema_version: 1,
        canonical_task_root_identity: fixture
            .task_root
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap()
            .display()
            .to_string(),
        expected_artifact_paths: archon_workflow::RUN_END_OBSERVER_EXPECTED_ARTIFACT_PATHS
            .into_iter()
            .map(str::to_string)
            .collect(),
        portable_acceptance_identity: None,
        lineage_recording: None,
    }
}

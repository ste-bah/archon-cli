//! The acceptance stage host call over a frozen fixture, direct mode.

use super::*;
use archon_workflow::acceptance_scratch::ScratchPolicy;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceCheck, AcceptanceContract,
    AcceptanceCriterion, AcceptanceLock, AcceptancePin, FreezeGateMode, FreezeGateStamp, GapPolicy,
    JudgeDecision, JudgeVerdict, PrdIdentity, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
    TrustedCwd, content_digest, empty_gate_findings_digest,
};
use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton, TaskSkeletonLock};
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::acceptance_stage::latest_round_record;
use archon_workflow::{RunStatus, WorkflowSpec, WorkflowV2HostCall, WorkflowV2HostMethod};

fn stamp() -> FreezeGateStamp {
    FreezeGateStamp {
        mode: FreezeGateMode::Observe,
        finding_count: 0,
        findings_digest: empty_gate_findings_digest(),
        binary_commit: "test-revision".into(),
        evaluated_at: "2026-08-27T00:00:00Z".into(),
    }
}

fn criterion(id: &str, command: &str) -> AcceptanceCriterion {
    AcceptanceCriterion {
        id: id.into(),
        criterion: format!("criterion {id}"),
        check: AcceptanceCheck::Command {
            command: command.into(),
            cwd: TrustedCwd::RepoRoot,
        },
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

struct Fixture {
    project: tempfile::TempDir,
    repo: tempfile::TempDir,
    task_root: std::path::PathBuf,
    store: WorkflowStore,
    runtime: WorkflowV2ScriptRuntime,
    universe: WorkflowV2TaskUniverse,
    run_id: String,
}

/// A project with a frozen two-task set whose repository has `present` but
/// not `missing`: REQ-1 passes, REQ-2 (owned by TASK-F-002) fails, REQ-9 (no
/// owner) fails.
fn fixture(freeze: bool) -> Fixture {
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
    for id in ["TASK-F-001", "TASK-F-002"] {
        std::fs::write(task_root.join(format!("{id}.md")), format!("# {id}\n")).unwrap();
    }
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
        acceptance: vec![
            criterion("REQ-1", "test -f present"),
            criterion("REQ-2", "test -f missing"),
            criterion("REQ-9", "test -f also-missing"),
        ],
        supplementary: Vec::new(),
    };
    let bytes = serde_json::to_vec_pretty(&contract).unwrap();
    std::fs::write(task_root.join(ACCEPTANCE_CONTRACT_FILE), &bytes).unwrap();
    if freeze {
        // The full frozen chain: lock, skeleton, skeleton lock and pin, so the
        // guardian's chain validation accepts the fixture too.
        let acceptance_digest = content_digest(&bytes);
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
        let frozen = |id: &str, implements: &[&str]| FrozenTask {
            task_id: id.into(),
            file_name: format!("{id}.md"),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: implements.iter().map(|s| s.to_string()).collect(),
            deliverable_contracts: Vec::new(),
        };
        let skeleton = TaskSkeleton {
            schema_version: 1,
            acceptance_digest: acceptance_digest.clone(),
            tasks: vec![
                frozen("TASK-F-001", &["REQ-1"]),
                frozen("TASK-F-002", &["REQ-2", "REQ-9"]),
            ],
        };
        let skeleton_bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
        let skeleton_digest = content_digest(&skeleton_bytes);
        std::fs::write(task_root.join(TASK_SKELETON_FILE), skeleton_bytes).unwrap();
        std::fs::write(
            task_root.join(TASK_SKELETON_LOCK_FILE),
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
            acceptance_digest,
            freeze_event_id: "freeze-fixture".into(),
            acceptance_gate: stamp(),
            skeleton_digest: Some(skeleton_digest),
            skeleton_gate: Some(stamp()),
            fidelity_waivers: Vec::new(),
        };
        let pin_path =
            crate::command::workflow_task_set::acceptance_pin_path(project.path(), &task_root);
        std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
        std::fs::write(pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    }
    let task = |id: &str, implements: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: task_root.join(format!("{id}.md")).display().to_string(),
        implements: implements.iter().map(|s| s.to_string()).collect(),
        ..WorkflowV2TaskUniverseTask::default()
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![task_root.display().to_string()],
        tasks: vec![
            task("TASK-F-001", &["REQ-1"]),
            task("TASK-F-002", &["REQ-2"]),
        ],
    };
    let store = WorkflowStore::project(project.path());
    let run = store
        .create_run(WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "acceptance".into(),
            task: "acceptance".into(),
            target_repository_root: Some(repo.path().display().to_string()),
            max_parallelism: 1,
            max_agents: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    let runtime = WorkflowV2ScriptRuntime {
        target_repository_root: Some(repo.path().display().to_string()),
        generated_config: Default::default(),
    };
    Fixture {
        run_id: run.id,
        project,
        repo,
        task_root,
        store,
        runtime,
        universe,
    }
}

fn execution(round: u32, max_rounds: u32, check_ids: &[&str]) -> WorkflowV2CallExecution {
    let (options, _) = archon_workflow::v2::script::parse_script_options(&serde_json::json!({
        "tool": ACCEPTANCE_STAGE_TOOL,
        "round": round,
        "maxRounds": max_rounds,
        "checkIds": check_ids,
    }))
    .unwrap();
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: format!("acceptance-contract-run-{round}"),
            method: WorkflowV2HostMethod::Tool,
            write_mode: None,
            options,
        },
        input: serde_json::json!({}),
        depends_on: Vec::new(),
    }
}

async fn run(
    fixture: &Fixture,
    execution: &WorkflowV2CallExecution,
) -> WorkflowResult<WorkflowV2Result> {
    run_acceptance_stage(
        &fixture.runtime,
        execution,
        &fixture.store,
        &fixture.run_id,
        Some(&fixture.universe),
    )
    .await
}

fn failing_ids(result: &WorkflowV2Result) -> Vec<String> {
    result.data["failing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| check["check_id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_first_round_records_every_check_with_its_owning_tasks() {
    let fixture = fixture(true);
    let call = execution(1, 3, &[]);
    assert!(is_acceptance_stage_call(&call));
    let result = run(&fixture, &call).await.expect("round runs");
    assert_eq!(
        result.status,
        WorkflowV2Status::Accepted,
        "remediation may still follow"
    );
    assert_eq!(result.data["final"], false);
    assert_eq!(result.data["contract_present"], true);
    assert_eq!(result.data["execution_mode"], "direct");
    assert_eq!(failing_ids(&result), vec!["REQ-2", "REQ-9"]);
    let failing = result.data["failing"].as_array().unwrap();
    assert_eq!(
        failing[0]["owning_tasks"],
        serde_json::json!(["TASK-F-002"])
    );
    assert_eq!(failing[1]["owning_tasks"], serde_json::json!([]));
    assert_eq!(
        result.data["unowned_failing_check_ids"],
        serde_json::json!(["REQ-9"])
    );
    assert_eq!(result.data["passed"], serde_json::json!(["REQ-1"]));
    assert!(result.validate().is_ok(), "{result:?}");
    let (record, path) = latest_round_record(&fixture.store.run_dir(&fixture.run_id))
        .unwrap()
        .expect("record written");
    assert_eq!((record.round, record.attempt), (1, 1));
    assert_eq!(record.checks.len(), 3);
    assert!(!record.final_round);
    let execution = record.execution.expect("execution site recorded");
    assert_eq!(execution.mode, "direct");
    assert!(!execution.config_present);
    assert!(
        execution
            .environment
            .contains("no [workflow.acceptance_execution]")
    );
    assert!(
        execution
            .source_commit
            .is_some_and(|commit| commit.len() == 40)
    );
    assert_eq!(
        result.data["record_path"],
        relative_record_path(&fixture.store.run_dir(&fixture.run_id), &path)
    );
    assert!(
        path.with_file_name("attempt-01")
            .join("REQ-2.stderr")
            .exists()
    );
}

/// The last permitted round that still fails answers `NeedsReview` and is
/// final; a clean round is `Accepted` and final.
#[tokio::test]
async fn terminal_rounds_answer_by_their_verdict() {
    let fixture = fixture(true);
    let last = run(&fixture, &execution(3, 3, &[])).await.unwrap();
    assert_eq!(last.status, WorkflowV2Status::NeedsReview);
    assert_eq!(last.data["final"], true);
    assert!(
        last.residual_gaps
            .iter()
            .any(|gap| gap.id == "acceptance-REQ-2")
    );
    // Only unowned failures left: nothing a task could fix, so the round is final.
    std::fs::write(fixture.repo.path().join("missing"), "x").unwrap();
    let unowned = run(&fixture, &execution(1, 3, &["REQ-2"])).await.unwrap();
    assert_eq!(failing_ids(&unowned), vec!["REQ-9"]);
    assert_eq!(unowned.status, WorkflowV2Status::NeedsReview);
    assert_eq!(unowned.data["final"], true);
    std::fs::write(fixture.repo.path().join("also-missing"), "x").unwrap();
    let clean = run(&fixture, &execution(1, 3, &["REQ-9"])).await.unwrap();
    assert_eq!(clean.status, WorkflowV2Status::Accepted);
    assert_eq!(clean.data["final"], true);
    assert!(failing_ids(&clean).is_empty());
}

#[tokio::test]
async fn re_entering_a_round_appends_a_new_attempt() {
    let fixture = fixture(true);
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    let dir = round_dir(&fixture.store.run_dir(&fixture.run_id), 1);
    assert!(dir.join("attempt-01.json").exists());
    assert!(dir.join("attempt-02.json").exists());
    let (record, _) = latest_round_record(&fixture.store.run_dir(&fixture.run_id))
        .unwrap()
        .unwrap();
    assert_eq!(record.attempt, 2);
}

/// A paused run unwinds the stage without a record; resuming re-enters it.
#[tokio::test]
async fn a_paused_run_interrupts_the_stage_and_resume_re_enters_it() {
    let fixture = fixture(true);
    let mut state = fixture.store.load_state(&fixture.run_id).unwrap();
    state.status = RunStatus::Paused;
    fixture.store.save_state(&state).unwrap();
    let error = run(&fixture, &execution(1, 3, &[]))
        .await
        .expect_err("a paused run stops the stage");
    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    assert!(
        latest_round_record(&fixture.store.run_dir(&fixture.run_id))
            .unwrap()
            .is_none(),
        "an interrupted round leaves no record to replay"
    );
    let mut state = fixture.store.load_state(&fixture.run_id).unwrap();
    state.status = RunStatus::Running;
    fixture.store.save_state(&state).unwrap();
    let result = run(&fixture, &execution(1, 3, &[]))
        .await
        .expect("re-entered");
    assert_eq!(failing_ids(&result), vec!["REQ-2", "REQ-9"]);
    let (record, _) = latest_round_record(&fixture.store.run_dir(&fixture.run_id))
        .unwrap()
        .unwrap();
    assert_eq!((record.round, record.attempt), (1, 1));
}

#[tokio::test]
async fn an_unfrozen_or_lost_contract_cannot_evaluate_and_an_undeclared_one_has_nothing_to_check() {
    let unfrozen = fixture(false);
    let result = run(&unfrozen, &execution(1, 3, &[])).await.unwrap();
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(result.summary.contains("not frozen"), "{}", result.summary);
    let (record, _) = latest_round_record(&unfrozen.store.run_dir(&unfrozen.run_id))
        .unwrap()
        .unwrap();
    assert!(record.blocks_completion());
    assert!(record.final_round);

    // Tasks that name the checks they implement declare a contract: losing
    // it cannot pass vacuously.
    let lost = fixture(false);
    std::fs::remove_file(lost.task_root.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    let result = run(&lost, &execution(1, 3, &[])).await.unwrap();
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(
        result.summary.contains("declares an acceptance contract"),
        "{}",
        result.summary
    );

    // A task set that never declared one has nothing to check.
    let mut absent = fixture(false);
    std::fs::remove_file(absent.task_root.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    for task in &mut absent.universe.tasks {
        task.implements.clear();
    }
    let result = run(&absent, &execution(1, 3, &[])).await.unwrap();
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(result.data["contract_present"], false);
    assert!(
        result.summary.contains("nothing to check"),
        "{}",
        result.summary
    );
}

/// The scratch guardian narrows an observation to the requested pinned
/// checks, refuses an id outside the pinned chain, and carries the selection
/// on the request line without changing the R2 request struct.
#[test]
fn the_scratch_guardian_narrows_an_observation_to_the_requested_checks() {
    use crate::command::acceptance_scratch_guardian::{
        Request, parse_request_line, request_line, validate_selected,
    };
    let fixture = fixture(true);
    let scratch = tempfile::tempdir().unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(
        fixture.project.path(),
        &fixture.task_root,
    );
    let request = Request {
        policy: ScratchPolicy {
            repository: fixture.repo.path().canonicalize().unwrap(),
            project: fixture.project.path().canonicalize().unwrap(),
            task_root: fixture.task_root.canonicalize().unwrap(),
            scratch_parent: scratch.path().to_path_buf(),
            project_inputs: vec![],
            project_input_excludes: vec![],
            combined: false,
            toolchain_path: "/usr/bin:/bin".into(),
            environment: Default::default(),
            environment_allowlist: vec![],
            cargo_seed: None,
            timeout_secs: 60,
            output_bytes: 2048,
            scratch_bytes: 16_777_216,
        },
        source_commit: "0".repeat(40),
        pin_path: pin_path.clone(),
        expected_pin_digest: content_digest(&std::fs::read(&pin_path).unwrap()),
        evidence: scratch.path().join("evidence"),
    };
    let ids = |refs: &[archon_workflow::acceptance_world::FrozenCommandRef]| {
        refs.iter()
            .map(|r| r.acceptance_id.clone())
            .collect::<Vec<_>>()
    };
    let (_, _, all) = validate_selected(&request, &None).expect("whole chain");
    assert_eq!(ids(&all), vec!["REQ-1", "REQ-2", "REQ-9"]);
    let selection = Some(std::collections::BTreeSet::from(["REQ-2".to_string()]));
    let (_, _, narrowed) = validate_selected(&request, &selection).expect("narrowed");
    assert_eq!(ids(&narrowed), vec!["REQ-2"]);
    let error = validate_selected(
        &request,
        &Some(std::collections::BTreeSet::from(["REQ-404".to_string()])),
    )
    .expect_err("an id outside the chain is refused");
    assert!(
        error.to_string().contains("not in the pinned contract"),
        "{error}"
    );
    let line = request_line(&request, &selection).unwrap();
    let (parsed, carried) = parse_request_line(std::str::from_utf8(&line).unwrap()).unwrap();
    assert_eq!(carried, selection);
    assert_eq!(parsed.evidence, request.evidence);
    let plain = request_line(&request, &None).unwrap();
    let (parsed, carried) = parse_request_line(std::str::from_utf8(&plain).unwrap()).unwrap();
    assert_eq!(carried, None);
    assert_eq!(parsed.pin_path, request.pin_path);
}

// Contract coverage of every round; split out to hold the 500-line ceiling.
#[path = "workflow_live_v3_acceptance_tests_b.rs"]
mod contract_coverage;

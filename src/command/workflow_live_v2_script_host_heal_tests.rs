//! Recovery inside one persisted capability call, through trusted dispatch.
use super::*;
use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, HostCommandProcessAdapter,
};
use crate::command::workflow_host_command_supervisor::{
    HostCommandControl, SupervisedProcessOutput,
};

struct RecoveringProcess {
    calls: AtomicUsize,
    failures: usize,
    permanent: bool,
    started: tokio::sync::Notify,
    incomplete: bool,
    scoped: bool,
    evaluation: std::sync::Mutex<Option<crate::command::workflow_gate::GateEvaluation>>,
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for RecoveringProcess {
    async fn execute(
        &self,
        request: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        _control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        if self.permanent {
            return Err(WorkflowError::Io {
                path: request.program,
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            });
        }
        if self.incomplete && attempt < self.failures {
            let stdout = serde_json::to_vec(&archon_workflow::GateOperationalError {
                kind: "transient_host_fault".into(),
                text: "the host observation was deferred".into(),
            })
            .unwrap();
            return Ok(SupervisedProcessOutput {
                exit_code: Some(75),
                timed_out: false,
                stdout_bytes: stdout.len() as u64,
                stdout_retained_bytes: stdout.len() as u64,
                stdout,
                stderr:
                    b"operational: the host could not prove 1 check(s): AC-X-001: connection reset"
                        .to_vec(),
                stderr_bytes: 84,
                stderr_retained_bytes: 84,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_path: None,
                stderr_path: None,
            });
        }
        let envelope = request
            .declared_write_set
            .iter()
            .find(|path| path.file_name().unwrap() == "gate-envelope.json")
            .unwrap();
        let call_id = request
            .args
            .windows(2)
            .find(|pair| pair[0] == "--call-id")
            .unwrap()[1]
            .clone();
        let mut evaluation =
            crate::command::workflow_gate::GateEvaluation::new("accepted", Vec::new());
        if attempt < self.failures {
            if self.scoped {
                evaluation.findings.push(
                    crate::command::workflow_gate::GateFinding::new(
                        crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
                        "operational: the host could not prove AC-X-001: connection reset",
                        "AC-X-001",
                        None,
                        archon_workflow::RemediationScope::Operational,
                    )
                    .with_defect(
                        archon_workflow::defect::DeterministicDefect::new(
                            "host_transient",
                            "AC-X-001",
                            "check",
                        ),
                    ),
                );
            } else {
                evaluation = evaluation.with_host_error(
                    &anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionReset))
                        .context("judge transport"),
                );
            }
        }
        if let Some(injected) = self.evaluation.lock().unwrap().take() {
            evaluation = injected;
        }
        let manifest = crate::command::workflow_gate_envelope::stage_gate_evaluation(
            envelope.parent().unwrap(),
            envelope,
            &call_id,
            &request.command_id,
            evaluation,
            Vec::new(),
        )
        .unwrap();
        let stdout = serde_json::to_vec(&manifest).unwrap();
        Ok(SupervisedProcessOutput {
            exit_code: Some(0),
            timed_out: false,
            stdout_bytes: stdout.len() as u64,
            stdout_retained_bytes: stdout.len() as u64,
            stdout,
            stderr: Vec::new(),
            stderr_bytes: 0,
            stderr_retained_bytes: 0,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_path: None,
            stderr_path: None,
        })
    }
}

struct HealFixture {
    _temp: tempfile::TempDir,
    store: WorkflowStore,
    run_id: String,
    executor: Arc<FixedHostCommandExecutor>,
    process: Arc<RecoveringProcess>,
}

fn fixture(failures: usize, permanent: bool) -> HealFixture {
    fixture_mode(failures, permanent, false, false)
}

fn fixture_mode(failures: usize, permanent: bool, incomplete: bool, scoped: bool) -> HealFixture {
    let temp = tempfile::tempdir().unwrap();
    let mut context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let input = "## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | The behavior is verified. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The result is observable. |\n";
    std::fs::write(&context.prd_path, input).unwrap();
    context.prd_digest = archon_workflow::task_set_contract::content_digest(input.as_bytes());
    crate::command::workflow_host_command_exec_tests::seed_frozen_subjects(
        &context,
        &[("TASK-X-001", "TASK-X-001.md")],
    );
    std::fs::write(context.task_root.join("TASK-X-001.md"), "# Candidate\n\n```yaml\ntask_id: TASK-X-001\ntitle: Candidate\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Focused Tests\n- `test -f TASK-X-001.md`\n").unwrap();
    let store = WorkflowStore::project(&context.project_root);
    let run = store.create_run(test_spec()).unwrap();
    super::workflow_live_v2_script_fixed_progress_tests::seed_fixed_progress_state(
        &store,
        &run.id,
        &context.task_root.join(".decompose.log"),
    );
    context.run_staging_root = store.run_dir(&run.id).join("host-command-staging");
    let process = Arc::new(RecoveringProcess {
        calls: AtomicUsize::new(0),
        failures,
        permanent,
        started: tokio::sync::Notify::new(),
        incomplete,
        scoped,
        evaluation: Default::default(),
    });
    let executor = Arc::new(FixedHostCommandExecutor::with_process(
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("revision")
            .unwrap(),
        context,
        store.run_dir(&run.id),
        process.clone(),
    ));
    HealFixture {
        _temp: temp,
        store,
        run_id: run.id,
        executor,
        process,
    }
}

async fn run(fixture: &HealFixture) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
    run_script(fixture, r#"async function workflow(w) { return await w.hostCommand("task-set-lint", { stdin: null }); }"#).await
}

async fn run_script(
    fixture: &HealFixture,
    source: &str,
) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
    let (ui, _rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui,
        Vec::new(),
        fixture.run_id.clone(),
        None,
        None,
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        WorkflowV2ScriptRunner::new(
            "capability".into(),
            test_runtime(&test_spec()),
            WorkflowV2AgentAdapter::new(),
            client,
            WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2")),
            fixture.store.clone(),
            fixture.run_id.clone(),
            true,
            None,
            None,
        )
        .with_host_command_executor(fixture.executor.clone())
        .run(source),
    )
    .await
    .expect("host recovery must finish or receive control")
}

fn records(fixture: &HealFixture) -> Vec<WorkflowV2CallRecord> {
    WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"))
        .load_call_records()
        .unwrap()
}

#[tokio::test]
async fn host_command_transient_gate_recovers_before_recording() {
    let fixture = fixture(3, false);
    run(&fixture).await.unwrap();
    let records = records(&fixture);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 4);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].attempt, 1);
    assert!(records[0].result.data["gateEnvelope"]["operational_error"].is_null());
    assert!(records[0].result.data["publicationReceipt"].is_object());
}

#[tokio::test]
async fn host_command_missing_program_returns_once_with_remedy() {
    let fixture = fixture(0, true);
    run(&fixture).await.unwrap();
    let records = records(&fixture);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    let error = &records[0].result.data["gateEnvelope"]["operational_error"];
    assert_eq!(error["kind"], "permanent_host_fault");
    assert!(error["text"].as_str().unwrap().contains("/trusted/"));
    assert!(error["text"].as_str().unwrap().contains("install"));
}

#[tokio::test(start_paused = true)]
async fn host_command_cancel_interrupts_backoff() {
    let fixture = fixture(usize::MAX, false);
    let work = run(&fixture);
    tokio::pin!(work);
    for attempt in 1..=3 {
        tokio::select! {
            biased;
            result = &mut work => panic!("transient call escaped before cancellation: {result:?}"),
            _ = fixture.process.started.notified() => {},
        }
        assert_eq!(fixture.process.calls.load(Ordering::SeqCst), attempt);
        if attempt < 3 {
            tokio::time::advance(std::time::Duration::from_millis(500 * attempt as u64)).await;
        }
    }
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Cancel)
        .unwrap();
    let before = tokio::time::Instant::now();
    // The script worker is a blocking task, so drive the existing control poll
    // explicitly while the test clock is paused.
    tokio::time::advance(std::time::Duration::from_millis(100)).await;
    let error = work.await.unwrap_err();
    assert!(
        matches!(error, WorkflowError::ControlCancelled(_)),
        "{error:?}"
    );
    assert!(before.elapsed() <= std::time::Duration::from_secs(2));
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn host_command_replay_keeps_one_recovered_result_without_execution() {
    let fixture = fixture(2, false);
    run(&fixture).await.unwrap();
    let before = records(&fixture);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 3);
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    run(&fixture).await.unwrap();
    let after = records(&fixture);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 3);
    assert_eq!(before.len(), 1);
    assert_eq!(after.len(), 1);
    assert_eq!(
        serde_json::to_value(&before[0]).unwrap(),
        serde_json::to_value(&after[0]).unwrap()
    );
}

#[tokio::test]
async fn reverify_host_command_transient_judge_recovers() {
    let fixture = fixture(2, false);
    let generation = fixture
        .store
        .load_state(&fixture.run_id)
        .unwrap()
        .generation;
    let request = archon_workflow::HostCommandRequest::new("task-set-lint", None).unwrap();
    let result = fixture
        .executor
        .execute_observed_only(request, Some(generation))
        .await
        .unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 3);
    assert!(result.gate_envelope.unwrap().operational_error.is_none());
    assert!(result.observed_only);
}

#[tokio::test]
async fn host_command_unproven_transport_recovers_before_policy_pause() {
    let fixture = fixture_mode(3, false, true, false);
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 4);
    assert!(records(&fixture)[0].result.data["publicationReceipt"].is_object());
}

#[tokio::test]
async fn host_command_operational_scope_recovers_before_recording() {
    let fixture = fixture_mode(3, false, false, true);
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 4);
    assert!(
        records(&fixture)[0].result.data["gateEnvelope"]["policy_findings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[path = "workflow_host_command_heal_safety_tests.rs"]
mod workflow_host_command_heal_safety_tests;

#[tokio::test]
async fn host_command_distinct_occurrences_execute_identical_inputs_independently() {
    let fixture = fixture(0, false);
    run_script(
        &fixture,
        r#"async function workflow(w) {
        await w.hostCommand("task-set-lint", { stdin: null });
        return await w.hostCommand("task-set-lint", { stdin: null });
    }"#,
    )
    .await
    .unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
    assert_eq!(records(&fixture).len(), 2);
}

//! The fixed-run boundary durably pauses unplanned errors without finalization.
use super::*;
use crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;
use archon_workflow::{HostCommandRequest, HostCommandResult, WorkflowV2CallRecord};

struct UnusedHost;

#[async_trait::async_trait]
impl WorkflowHostCommandExecutor for UnusedHost {
    fn call_identity(&self, _: &HostCommandRequest) -> archon_workflow::WorkflowResult<String> {
        Err(WorkflowError::SpecInvalid("host identity fault".into()))
    }

    fn record_is_reusable(
        &self,
        _: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(false)
    }

    async fn execute(
        &self,
        _: HostCommandRequest,
        _: Option<u64>,
    ) -> archon_workflow::WorkflowResult<HostCommandResult> {
        panic!("no host process should run")
    }
}

struct UnusedLlm;

#[async_trait::async_trait]
impl WorkflowLlmClient for UnusedLlm {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        panic!("no provider should run")
    }
}

async fn fixed_pause(source: &str, expected: &str) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "fixed-error-probe".into(),
            task: "test fixed script error control".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let plan =
        WorkflowScriptPlan::fixed(run.spec.clone(), source, Vec::new(), serde_json::json!({}));
    let (sink, _ui) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let outcome = execute_fixed_decomposition_v2_run(
        &store,
        run.clone(),
        plan,
        Arc::new(UnusedLlm),
        sink,
        Vec::new(),
        Arc::new(UnusedHost),
    )
    .await;
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Paused,
        "{outcome:?}"
    );
    let report = outcome.unwrap();
    assert!(report.contains(expected), "{report}");
    assert!(
        !store
            .run_dir(&run.id)
            .join(super::super::workflow_live_v2_finalizer::FINALIZATION_RECORD_PATH)
            .exists()
    );
}

#[tokio::test]
async fn fixed_boundary_runtime_error_pauses_without_terminal_finalization() {
    fixed_pause(
        "async function workflow(w) { throw new TypeError('unplanned defect'); }",
        "unplanned defect",
    )
    .await;
}

#[tokio::test]
async fn fixed_boundary_syntax_error_pauses_without_terminal_finalization() {
    fixed_pause(
        "async function workflow(w) { const invalid = ; }",
        "unexpected token",
    )
    .await;
}

#[tokio::test]
async fn fixed_boundary_host_error_pauses_without_terminal_finalization() {
    fixed_pause(
        "async function workflow(w) { await w.hostCommand('inspect'); }",
        "host identity fault",
    )
    .await;
}

/// The first gate refuses (unpublished); the second faults the host itself
/// while `fault` is set, past the script-error pause, up to the run boundary.
struct RefuseThenFault {
    fault: bool,
    refusals: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl WorkflowHostCommandExecutor for RefuseThenFault {
    fn call_identity(
        &self,
        request: &HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}:fixed", request.command_id))
    }

    fn record_is_reusable(
        &self,
        _: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(true)
    }

    async fn execute(
        &self,
        request: HostCommandRequest,
        _: Option<u64>,
    ) -> archon_workflow::WorkflowResult<HostCommandResult> {
        if request.command_id != "task-set-lint" {
            assert!(!self.fault, "host fault outside every script-error path");
        } else {
            self.refusals
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(HostCommandResult {
            exit_code: Some(1),
            stdout: "refused".into(),
            stderr: String::new(),
            stdout_bytes: 7,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: None,
            publication_receipt: None,
            subjects: Vec::new(),
            postcondition: None,
        })
    }
}

/// Issue 337: the boundary pause covers what the run recorded, so a resume
/// replays the judge's refusal instead of re-asking it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fixed_boundary_pause_replays_recorded_refusal_on_resume() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "fixed-boundary-probe".into(),
            task: "test fixed boundary coverage".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let source = r#"async function workflow(w) {
      const gate = await w.hostCommand("task-set-lint", { stdin: null });
      await w.hostCommand("task-set-gate", { stdin: null });
      return gate.stdout;
    }"#;
    let refusals = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for fault in [true, false] {
        let current = store.load_state(&run.id).unwrap();
        let plan = WorkflowScriptPlan::fixed(
            current.spec.clone(),
            source,
            Vec::new(),
            serde_json::json!({}),
        );
        let (sink, _ui) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
        let outcome = execute_fixed_decomposition_v2_run(
            &store,
            current,
            plan,
            Arc::new(UnusedLlm),
            sink,
            Vec::new(),
            Arc::new(RefuseThenFault {
                fault,
                refusals: refusals.clone(),
            }),
        )
        .await;
        if fault {
            assert_eq!(
                store.load_state(&run.id).unwrap().status,
                RunStatus::Paused,
                "{outcome:?}"
            );
            assert!(outcome.unwrap().contains("paused"));
            archon_workflow::LifecycleController::new(store.clone())
                .apply(&run.id, archon_workflow::LifecycleAction::Resume)
                .unwrap();
        }
    }
    assert_eq!(
        refusals.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the refusal is replayed, never re-asked"
    );
}

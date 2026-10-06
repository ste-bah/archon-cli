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

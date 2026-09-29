//! Issue-136: a pause stops a read-only call whose model request is still in
//! flight within seconds, and the call is recorded as interrupted so a
//! resume dispatches it again. Before the race, the pause was seen only when
//! the model answered: here, never.

use super::*;

use std::sync::atomic::{AtomicBool, Ordering};

const PAUSE_PROBE_SCRIPT: &str = r#"
export const meta = { name: 'pause-demo', phases: [{ title: 'Batch' }] }

phase('Batch')
const batch = await agents(
  [{ prompt: 'Inspect the area and report.', label: 'branch-one' }],
  { maxParallelism: 1 }
)
return { batch_status: batch && batch.status }
"#;

/// A model request that does not come back for a minute and a half: far
/// past the bound, yet short enough that a host which never stops it still
/// returns and fails the timing assertion rather than hanging the suite.
struct StuckLlm {
    entered: std::sync::Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for StuckLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.entered.store(true, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        Err(WorkflowError::StageFailed(
            "the stuck request returned".into(),
        ))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pause_stops_an_in_flight_read_only_call_within_seconds() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "paused-call-test".to_string(),
            task: "test".to_string(),
            target_repository_root: None,
            max_parallelism: 2,
            max_agents: 4,
            stages: Vec::new(),
            permissions: std::collections::BTreeMap::new(),
            learning_hooks: Vec::new(),
        })
        .expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let entered = std::sync::Arc::new(AtomicBool::new(false));
    let client = LiveV2AgentClient::new(
        std::sync::Arc::new(StuckLlm {
            entered: entered.clone(),
        }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let pauser = {
        let store = workflow_store.clone();
        let run_id = run.id.clone();
        tokio::spawn(async move {
            while !entered.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            archon_workflow::LifecycleController::new(store)
                .apply(&run_id, archon_workflow::LifecycleAction::Pause)
                .expect("pause the run while the model request is in flight");
            std::time::Instant::now()
        })
    };
    let runner = WorkflowV2ScriptRunner::new(
        "paused call".to_string(),
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: archon_core::config::GeneratedWorkflowConfig::default(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        runner.run(PAUSE_PROBE_SCRIPT),
    )
    .await
    .expect("the paused run kept waiting on its in-flight model request")
    .expect_err("a paused run must unwind the script");
    let paused_at = pauser.await.expect("pauser");
    assert!(
        paused_at.elapsed() < std::time::Duration::from_secs(10),
        "the call ended {:?} after the pause",
        paused_at.elapsed()
    );
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "the caller sees the pause: {error:?}"
    );
    let record = v2_store
        .load_call_record("agents-1")
        .expect("record lookup")
        .expect("the stopped call leaves a record");
    assert_eq!(
        record
            .result
            .data
            .get("interrupted")
            .and_then(|v| v.as_str()),
        Some("paused"),
        "recorded as interrupted, so a resume dispatches it again: {}",
        record.result.data
    );
    assert!(!is_reusable_status(record.status));
}

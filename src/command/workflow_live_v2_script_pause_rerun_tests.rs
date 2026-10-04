//! Issue-258: status reads a paused call as interrupted, but its record stays
//! `NeedsReview` on purpose. This pins the consequence: a call a pause stopped
//! mid-flight is dispatched again by the resume, never reused.

use super::*;

use std::sync::atomic::{AtomicBool, Ordering};

use super::workflow_live_v2_script_control_tests::{StuckLlm, create_run};

const RERUN_PROBE_SCRIPT: &str = r#"
async function workflow(w) {
  const inspected = await w.agent("inspect-one", { role: "analysis", task: "Inspect the area and report." });
  return { inspected_status: inspected && inspected.status };
}
"#;

/// Answers at once, and says it was asked.
struct AnsweringLlm {
    entered: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for AnsweringLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.entered.store(true, Ordering::SeqCst);
        let mut result = WorkflowV2Result::accepted("inspected the area");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "read the area",
        ));
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: serde_json::to_string(&result).expect("result json"),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: None,
        })
    }
}

fn runner_with(
    store: &WorkflowStore,
    run_id: &str,
    llm: Arc<dyn archon_workflow::WorkflowLlmClient>,
) -> (WorkflowV2ScriptRunner, impl Sized) {
    let (ui_sink, ui) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run_id.to_string(), None, None);
    let runner = WorkflowV2ScriptRunner::new(
        "pause rerun probe".to_string(),
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: archon_core::config::GeneratedWorkflowConfig::default(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")),
        store.clone(),
        run_id.to_string(),
        true,
        None,
        None,
    );
    // The receiver lives as long as the runner: a dropped one closes the
    // channel and fails the call's status delivery.
    (runner, ui)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_paused_mid_flight_runs_again_on_resume() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = create_run(&store);
    let lifecycle = archon_workflow::LifecycleController::new(store.clone());

    // Session 1: the call is in flight when the pause lands.
    let entered = Arc::new(AtomicBool::new(false));
    let pauser = {
        let (lifecycle, run_id, entered) = (lifecycle.clone(), run.id.clone(), entered.clone());
        tokio::spawn(async move {
            while !entered.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            lifecycle
                .apply(&run_id, archon_workflow::LifecycleAction::Pause)
                .expect("pause");
        })
    };
    let (first, _ui) = runner_with(&store, &run.id, Arc::new(StuckLlm { entered }));
    let error = tokio::time::timeout(Duration::from_secs(300), first.run(RERUN_PROBE_SCRIPT))
        .await
        .expect("the pause ends the first session")
        .expect_err("a paused run unwinds the script");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    pauser.await.expect("pauser");
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let interrupted = v2_store
        .load_call_record("inspect-one")
        .expect("record lookup")
        .expect("the paused call left a record");
    assert_eq!(interrupted.result.data["interrupted"], "paused");
    assert_eq!(interrupted.status, WorkflowV2Status::NeedsReview);

    // Session 2: resumed. The call is dispatched again and its new answer
    // takes the slot.
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Resume)
        .expect("resume");
    let dispatched = Arc::new(AtomicBool::new(false));
    let (second, _ui) = runner_with(
        &store,
        &run.id,
        Arc::new(AnsweringLlm {
            entered: dispatched.clone(),
        }),
    );
    let summary = tokio::time::timeout(Duration::from_secs(60), second.run(RERUN_PROBE_SCRIPT))
        .await
        .expect("the resumed session finishes")
        .expect("the resumed session completes");
    assert!(
        dispatched.load(Ordering::SeqCst),
        "the paused call was dispatched again, not reused"
    );
    assert_eq!(summary.executed, 1, "{summary:?}");
    assert_eq!(summary.reused, 0, "{summary:?}");
    let rerun = v2_store
        .load_call_record("inspect-one")
        .expect("record lookup")
        .expect("the re-run call left a record");
    assert_eq!(
        (rerun.attempt, rerun.status),
        (2, WorkflowV2Status::Accepted)
    );
}

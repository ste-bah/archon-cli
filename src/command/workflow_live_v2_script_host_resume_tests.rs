//! Issue-250: a call's accepted record survives a later attempt that a pause
//! stopped. The resume that asks the accepted input again reuses that record
//! (no agent is dispatched) and puts it back into the call's slot.

use super::*;

use std::sync::atomic::{AtomicBool, Ordering};

const RESUME_PROBE_SCRIPT: &str = r#"
export const meta = { name: 'resume-demo', phases: [{ title: 'Only' }] }

phase('Only')
const inspected = await agent('Inspect the area and report.', { label: 'inspect-one' })
return { inspected_status: inspected && inspected.status }
"#;

/// A model request that does not come back until the test is long over:
/// only a pause ends the call.
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

fn runner_with(
    workflow_store: &WorkflowStore,
    run_id: &str,
    llm: std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>,
) -> (WorkflowV2ScriptRunner, impl Sized) {
    let (ui_sink, tui_rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run_id.to_string(), None, None);
    let runner = WorkflowV2ScriptRunner::new(
        "resume probe".to_string(),
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: archon_core::config::GeneratedWorkflowConfig::default(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(workflow_store.run_dir(run_id).join("v2")),
        workflow_store.clone(),
        run_id.to_string(),
        true,
        None,
        None,
    );
    (runner, tui_rx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_after_an_interrupted_rerun_reuses_the_accepted_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "resume-reuse-test".to_string(),
            task: "test".to_string(),
            target_repository_root: None,
            max_parallelism: 2,
            max_agents: 4,
            stages: Vec::new(),
            permissions: std::collections::BTreeMap::new(),
            learning_hooks: Vec::new(),
        })
        .expect("run");
    let lifecycle = archon_workflow::LifecycleController::new(workflow_store.clone());

    // Session 1: the call is dispatched and a pause stops it.
    let entered = std::sync::Arc::new(AtomicBool::new(false));
    let pauser = {
        let (lifecycle, run_id, entered) = (lifecycle.clone(), run.id.clone(), entered.clone());
        tokio::spawn(async move {
            while !entered.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            lifecycle
                .apply(&run_id, archon_workflow::LifecycleAction::Pause)
                .expect("pause");
        })
    };
    let stuck = std::sync::Arc::new(StuckLlm { entered });
    let (runner, _ui) = runner_with(&workflow_store, &run.id, stuck);
    let first = runner.run(RESUME_PROBE_SCRIPT);
    let error = tokio::time::timeout(std::time::Duration::from_secs(300), first)
        .await
        .expect("the pause ends the first session")
        .expect_err("a paused run unwinds the script");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    pauser.await.expect("pauser");

    // The disk as the issue found it: attempt 1 accepted (an earlier
    // session's answer to this very input), then attempt 2 stopped by the
    // pause, which archived attempt 1 and took the slot.
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let interrupted = v2_store
        .load_call_record("inspect-one-1")
        .expect("record lookup")
        .expect("the agent call left a record");
    assert_eq!(interrupted.result.data["interrupted"], "paused");
    let call_id = interrupted.call.id.clone();
    let mut accepted = interrupted.clone();
    accepted.status = WorkflowV2Status::Accepted;
    accepted.result = WorkflowV2Result::accepted("inspected the area");
    accepted.result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "read the area",
    ));
    accepted.started_at = "2026-10-01T10:00:00+00:00".to_string();
    accepted.finished_at = accepted.started_at.clone();
    v2_store
        .save_call_record(&accepted)
        .expect("accepted attempt 1");
    let mut second = interrupted.clone();
    second.attempt = 2;
    v2_store
        .save_call_record(&second)
        .expect("interrupted attempt 2");
    let slot = v2_store.load_call_record(&call_id).unwrap().unwrap();
    assert_eq!(
        (slot.attempt, slot.status),
        (2, WorkflowV2Status::NeedsReview)
    );
    let accepted = v2_store
        .last_accepted_call_record(&call_id, &accepted.input_hash)
        .unwrap()
        .expect("attempt 1 in the history");

    // Session 2: resumed with the same input. Any dispatch would hang the
    // stuck model, so the timeout also proves nothing was dispatched.
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Resume)
        .expect("resume");
    let dispatched = std::sync::Arc::new(AtomicBool::new(false));
    let never = std::sync::Arc::new(StuckLlm {
        entered: dispatched.clone(),
    });
    let (runner, _ui) = runner_with(&workflow_store, &run.id, never);
    let second_run = runner.run(RESUME_PROBE_SCRIPT);
    tokio::time::timeout(std::time::Duration::from_secs(60), second_run)
        .await
        .expect("the resumed session reuses instead of dispatching")
        .expect("the resumed session completes");
    assert!(
        !dispatched.load(Ordering::SeqCst),
        "no agent was dispatched"
    );

    // The accepted record is back in the slot, read from the file itself.
    let raw = std::fs::read_to_string(v2_store.result_path(&call_id)).expect("slot file");
    let restored: WorkflowV2CallRecord = serde_json::from_str(&raw).expect("slot json");
    assert_eq!(restored, accepted);
    assert_eq!(
        (restored.attempt, restored.status),
        (1, WorkflowV2Status::Accepted)
    );
    let archive = workflow_store
        .run_dir(&run.id)
        .join("v2/results/superseded");
    let archived = std::fs::read_dir(archive)
        .expect("archive")
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|raw| serde_json::from_str::<WorkflowV2CallRecord>(&raw).ok())
        .collect::<Vec<_>>();
    assert!(
        archived
            .iter()
            .any(|record| record.attempt == 2 && record.result.data["interrupted"] == "paused"),
        "the interrupted attempt stays in the history"
    );
    assert_eq!(v2_store.next_attempt(&call_id).unwrap(), 3);
}

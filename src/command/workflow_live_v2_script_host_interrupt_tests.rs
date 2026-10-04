//! A cancelled call must leave a record saying so.
//!
//! The regression: the `ControlPaused`/`ControlCancelled` arm in `execute`
//! returned above `save_call_record`, so a run stopped mid-call wrote nothing
//! to `v2/results/`. An authoring call cancelled two hours in left no trace of
//! what it had been doing — while a timeout or a transport error, which take
//! the `failed_v2_result` path, recorded normally.
//!
//! The test drives a real read-only fanout and cancels the run from inside the
//! first branch's provider call, so the cancel is observed by the branch's own
//! `poll_v2_run_control` — the same way an operator's `workflow cancel` reaches
//! a call already in flight.

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

/// A fanout of two branches: the host journals the whole call as `agents-1`.
const CANCEL_PROBE_SCRIPT: &str = r#"
export const meta = { name: 'interrupt-demo', phases: [{ title: 'Batch' }] }

phase('Batch')
const batch = await agents(
  [
    { prompt: 'Inspect the first independent area and report.', label: 'branch-one' },
    { prompt: 'Inspect the second independent area and report.', label: 'branch-two' },
  ],
  { maxParallelism: 2 }
)
return { batch_status: batch && batch.status }
"#;

/// Cancels the run from inside the first provider call, then answers normally.
///
/// Deterministic on purpose: the cancel is written before the branch returns,
/// so the branch's post-call control poll is guaranteed to observe it. Waiting
/// on wall-clock timing to cancel a concurrently running branch would race.
struct CancelMidCallLlm {
    store: archon_workflow::WorkflowStore,
    run_id: String,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for CancelMidCallLlm {
    /// Scripted replies stand for one continued session (#241).
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            archon_workflow::LifecycleController::new(self.store.clone())
                .apply(&self.run_id, archon_workflow::LifecycleAction::Cancel)
                .expect("cancel the run while the call is in flight");
        }
        let envelope = serde_json::json!({
            "status": "accepted",
            "summary": "branch inspected its item",
            "evidence": [{ "kind": "inspection", "summary": "read the named area" }],
        });
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: envelope.to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: None,
        })
    }
}

fn interrupt_test_spec() -> archon_workflow::WorkflowSpec {
    archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "interrupted-call-test".to_string(),
        task: "test".to_string(),
        target_repository_root: None,
        max_parallelism: 4,
        max_agents: 16,
        stages: Vec::new(),
        permissions: std::collections::BTreeMap::new(),
        learning_hooks: Vec::new(),
    }
}

#[tokio::test]
async fn a_cancelled_call_leaves_a_readable_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = interrupt_test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        std::sync::Arc::new(CancelMidCallLlm {
            store: workflow_store.clone(),
            run_id: run.id.clone(),
            calls: AtomicUsize::new(0),
        }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    // Captured before the store is moved into the runner: the run log is the
    // second half of what an interrupted call has to leave behind.
    let events_path = workflow_store.events_path(&run.id);
    let runner = WorkflowV2ScriptRunner::new(
        "interrupted call".to_string(),
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

    // Control flow is unchanged: the cancel still unwinds the script, and the
    // caller still sees the same error it saw before this fix.
    let error = runner
        .run(CANCEL_PROBE_SCRIPT)
        .await
        .expect_err("a cancelled run must unwind the script");
    assert!(
        matches!(error, WorkflowError::ControlCancelled(_)),
        "the caller must still see the untouched control error, got: {error:?}"
    );

    // ...and the call is no longer silent. The record round-trips through disk,
    // so this is what a human reading `v2/results/` after the incident gets.
    let record = v2_store
        .load_call_record("agents-1")
        .expect("call record lookup")
        .expect("a cancelled call must leave a record explaining itself");
    assert_eq!(
        record.status,
        WorkflowV2Status::NeedsReview,
        "a stopped call did not complete; it did not fail"
    );
    assert_eq!(record.call.id, "agents-1");
    assert_eq!(
        record
            .result
            .data
            .get("interrupted")
            .and_then(|v| v.as_str()),
        Some("cancelled"),
        "the record must say whether it was cancelled or paused: {}",
        record.result.data
    );
    assert!(
        record.result.summary.contains("agents-1") && record.result.summary.contains("cancelled"),
        "summary must name the call and what happened to it: {}",
        record.result.summary
    );
    assert!(
        record
            .result
            .data
            .get("elapsed_seconds")
            .is_some_and(serde_json::Value::is_u64),
        "elapsed must be a measured number, not absent or invented: {}",
        record.result.data
    );
    // The record must never be mistaken for work that can be replayed.
    assert!(
        !record.is_reusable_for(&record.input_hash),
        "an interrupted record must not be reusable as a result"
    );

    // The second half of the regression: the record existed and nothing
    // announced it. `events.jsonl` is what a resume, the board and an operator
    // read, so a call missing from it is invisible wherever anyone looks.
    let events = std::fs::read_to_string(&events_path).expect("run event log");
    let announcement = events
        .lines()
        .filter(|line| line.contains("agents-1"))
        .find(|line| line.contains("call_needs_review"))
        .unwrap_or_else(|| {
            panic!("no call_needs_review event for the interrupted call in:\n{events}")
        });
    assert!(
        announcement.contains("stage_stalled") || announcement.contains("StageStalled"),
        "an interrupted call is a stalled stage, not a completed one: {announcement}"
    );
}

#[tokio::test]
async fn stale_notification_failure_in_flight_preserves_a_fresh_first_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(interrupt_test_spec()).unwrap();
    archon_workflow::WorkflowBundle::create_for_run(
        &store,
        &run,
        "export default async function workflow(w) {}",
        archon_workflow::WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(CancelMidCallLlm {
            store: store.clone(),
            run_id: run.id.clone(),
            calls: AtomicUsize::new(0),
        }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let host = WorkflowScriptHost {
        scaffold_hash: "fixture".into(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner: WorkflowV2ScriptRunner::new(
            "interruption".into(),
            WorkflowV2ScriptRuntime {
                target_repository_root: None,
                generated_config: archon_core::config::GeneratedWorkflowConfig::default(),
            },
            WorkflowV2AgentAdapter::new(),
            client,
            v2.clone(),
            store.clone(),
            run.id.clone(),
            true,
            None,
            None,
        ),
        accumulator: Arc::new(tokio::sync::Mutex::new(WorkflowScriptAccumulator::default())),
        tool_host: std::sync::OnceLock::new(),
        tool_budget: Default::default(),
    };
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "inspect-one".into(),
            method: archon_workflow::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        input: serde_json::Value::Null,
        depends_on: vec![],
    };
    // The old dispatch starts before restart and reports delivery failure only
    // after the fresh session has committed its result.
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (failure_tx, failure_rx) = tokio::sync::oneshot::channel();
    let old_execution = execution.clone();
    let old = tokio::spawn(async move {
        let attempt = host
            .runner
            .v2_store
            .next_attempt(&old_execution.call.id)
            .unwrap();
        assert_eq!(attempt, 1);
        assert_eq!(host.fixed_execution_generation().unwrap(), None);
        started_tx.send(()).unwrap();
        let failure = failure_rx.await.unwrap();
        host.save_interrupted_call_record(
            &old_execution,
            NOTIFICATION_DELIVERY_REASON,
            &failure,
            std::time::Duration::from_secs(1),
            attempt,
            "in",
            None,
            None,
        )
        .await;
    });
    started_rx.await.unwrap();
    archon_workflow::v2::restart::invalidate_generated_v2_call(&store, &run, &execution.call.id)
        .unwrap();
    let fresh = WorkflowV2ResultStore::new(v2.root());
    assert_eq!(fresh.next_attempt(&execution.call.id).unwrap(), 1);
    let mut result = WorkflowV2Result::accepted("fresh post-restart result");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "area inspected",
    ));
    let accepted = WorkflowV2CallRecord::new(
        run.id.clone(),
        execution.call.clone(),
        1,
        "in".into(),
        result,
        vec![],
    );
    fresh.save_call_record(&accepted).unwrap();
    let before = std::fs::read(fresh.result_path(&execution.call.id)).unwrap();
    let events_before = std::fs::read(store.events_path(&run.id)).unwrap();
    failure_tx
        .send(WorkflowError::NotificationDelivery(
            "delivery failed".into(),
        ))
        .unwrap();
    old.await.unwrap();
    assert_eq!(
        std::fs::read(fresh.result_path(&execution.call.id)).unwrap(),
        before,
        "stale interruption overwrote the post-restart accepted attempt"
    );
    assert_eq!(
        std::fs::read(store.events_path(&run.id)).unwrap(),
        events_before
    );
    assert_eq!(
        fresh.load_call_record(&execution.call.id).unwrap().unwrap(),
        accepted
    );
}

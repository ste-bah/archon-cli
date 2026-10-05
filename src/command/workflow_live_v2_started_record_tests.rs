//! Issue 303: a started record whose publication failed never outlives its
//! executor as `Running`, and a resume dispatches the call exactly once.
use super::round7_terminal_tests::NoHostCommands;
use super::terminal_test_support::{save_fixed_metadata, seed_fixed};
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Answers every dispatch with an accepted result and counts them.
struct CountingReply(AtomicUsize);
#[async_trait::async_trait]
impl WorkflowLlmClient for CountingReply {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        let dispatch = self.0.fetch_add(1, Ordering::SeqCst);
        let mut result = WorkflowV2Result::accepted(format!("dispatch {dispatch}"));
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "read the requested area",
        ));
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: serde_json::to_string(&result)?,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: None,
        })
    }
}

/// The started event of call `fault` fails once; with `hang`, only once
/// `hang` is released, standing in for a host killed in that window.
struct StartedFault {
    hang: Option<Arc<tokio::sync::Notify>>,
    hit: AtomicBool,
}

/// Wakes the stalled first host when the test ends, however it ends, so the
/// test runtime can shut down; by then a newer executor owns the run.
struct Release(Arc<tokio::sync::Notify>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}
#[async_trait::async_trait]
impl archon_workflow::WorkflowUiSink for StartedFault {
    async fn emit(
        &self,
        event: archon_workflow::WorkflowUiEvent,
    ) -> archon_workflow::WorkflowUiResult {
        if let archon_workflow::WorkflowUiEvent::Activity(update) = event
            && update.id.ends_with(":fault")
            && !self.hit.swap(true, Ordering::SeqCst)
        {
            if let Some(hang) = &self.hang {
                hang.notified().await;
            }
            return Err(archon_workflow::WorkflowUiDeliveryError::new(
                "injected started delivery failure",
            ));
        }
        Ok(())
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    store: WorkflowStore,
    run: WorkflowRun,
    plan: WorkflowScriptPlan,
    v2: WorkflowV2ResultStore,
    llm: Arc<CountingReply>,
}

fn fixture(script: &str) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    seed_fixed(&store, &run.id, temp.path());
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    save_fixed_metadata(&store, &run.id, &plan);
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    Fixture {
        _temp: temp,
        store,
        run,
        plan,
        v2,
        llm: Arc::new(CountingReply(AtomicUsize::new(0))),
    }
}

async fn execute(fx: &Fixture, run: WorkflowRun, sink: SharedWorkflowUiSink) -> Result<String> {
    execute_fixed_decomposition_v2_run(
        &fx.store,
        run,
        fx.plan.clone(),
        fx.llm.clone(),
        sink,
        Vec::new(),
        Arc::new(NoHostCommands),
    )
    .await
}

fn assert_nothing_running(fx: &Fixture) {
    let records = fx.v2.load_call_records().unwrap();
    assert!(
        records
            .iter()
            .all(|r| r.status != WorkflowV2Status::Running),
        "{records:#?}"
    );
}

fn inflight_markers(fx: &Fixture) -> usize {
    std::fs::read_dir(fx.v2.root().join("inflight")).map_or(0, |dir| dir.count())
}

/// Every archived record of `call_id`.
fn history(fx: &Fixture, call_id: &str) -> Vec<WorkflowV2CallRecord> {
    let Ok(dir) = std::fs::read_dir(fx.v2.call_history_dir(call_id)) else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|entry| std::fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect()
}

#[tokio::test]
async fn workflow_live_v2_started_delivery_failure_then_throw_leaves_no_running_record() {
    let fx = fixture(
        r#"async function workflow(w) {
            await w.agent("fault", {role: "analysis", task: "Inspect the area."});
        }"#,
    );
    let sink = Arc::new(StartedFault {
        hang: None,
        hit: AtomicBool::new(false),
    });
    let report = execute(&fx, fx.run.clone(), sink.clone()).await;
    assert!(sink.hit.load(Ordering::SeqCst), "failure injected");
    let record = fx.v2.load_call_record("fault").unwrap().unwrap();
    assert_eq!(record.status, WorkflowV2Status::NeedsReview, "{report:?}");
    assert_eq!(
        record.result.data["interrupted"],
        "notification_delivery_failed"
    );
    assert_nothing_running(&fx);
    assert_eq!(fx.llm.0.load(Ordering::SeqCst), 0, "never dispatched");
    let state = fx.store.load_state(&fx.run.id).unwrap();
    assert!(
        state
            .stages
            .values()
            .all(|s| s.status != archon_workflow::StageStatus::Running),
        "{:?}",
        state.stages
    );
}

#[tokio::test]
async fn workflow_live_v2_started_delivery_failure_then_catch_continues_without_running_record() {
    let fx = fixture(
        r#"async function workflow(w) {
            try {
                await w.agent("fault", {role: "analysis", task: "Inspect the area."});
            } catch (_) {}
            await w.agent("next", {role: "analysis", task: "Inspect another area."});
        }"#,
    );
    let sink = Arc::new(StartedFault {
        hang: None,
        hit: AtomicBool::new(false),
    });
    let _ = execute(&fx, fx.run.clone(), sink.clone()).await;
    assert!(sink.hit.load(Ordering::SeqCst), "failure injected");
    let fault = fx.v2.load_call_record("fault").unwrap().unwrap();
    assert_eq!(fault.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        fault.result.data["interrupted"],
        "notification_delivery_failed"
    );
    let next = fx.v2.load_call_record("next").unwrap().unwrap();
    assert_eq!(next.status, WorkflowV2Status::Accepted);
    assert_eq!(fx.llm.0.load(Ordering::SeqCst), 1, "only `next` ran");
    assert_nothing_running(&fx);
}

/// The host dies after the started record is saved and before the call is
/// dispatched: no in-flight marker, a `Running` record. The resume closes
/// that record with its evidence, then dispatches the call exactly once.
#[tokio::test]
async fn workflow_live_v2_resume_after_started_crash_closes_record_and_runs_call_once() {
    let fx = fixture(
        r#"async function workflow(w) {
            await w.agent("fault", {role: "analysis", task: "Inspect the area."});
        }"#,
    );
    let notify = Arc::new(tokio::sync::Notify::new());
    let _release = Release(notify.clone());
    let hang = Arc::new(StartedFault {
        hang: Some(notify),
        hit: AtomicBool::new(false),
    });
    let crashed = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        execute(&fx, fx.run.clone(), hang.clone()),
    )
    .await;
    assert!(crashed.is_err(), "the host stops in the started window");
    assert!(hang.hit.load(Ordering::SeqCst));
    let left = fx.v2.load_call_record("fault").unwrap().unwrap();
    assert_eq!(left.status, WorkflowV2Status::Running, "crash state");
    assert_eq!(inflight_markers(&fx), 0, "dispatch never began");
    assert_eq!(fx.llm.0.load(Ordering::SeqCst), 0);

    // The dead-owner recovery and takeover a resume performs.
    let run = fx
        .store
        .with_run_lock(&fx.run.id, |locked| {
            let mut run = locked.load_state(&fx.run.id)?;
            run.status = RunStatus::Paused;
            run.generation += 1;
            run.executor_generation = Some(run.generation);
            locked.save_state(&run)?;
            Ok(run)
        })
        .unwrap();
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let report = execute(&fx, run, ui).await.unwrap();
    assert_eq!(fx.llm.0.load(Ordering::SeqCst), 1, "exactly once: {report}");
    let record = fx.v2.load_call_record("fault").unwrap().unwrap();
    assert_eq!(record.status, WorkflowV2Status::Accepted, "{report}");
    assert_nothing_running(&fx);
    // The crashed attempt was closed with its evidence before the re-run.
    let archived = history(&fx, "fault");
    let closed = archived
        .iter()
        .find(|r| r.result.data["interrupted"] == "dispatch_not_started")
        .unwrap_or_else(|| panic!("crashed attempt closed: {archived:#?}"));
    assert_eq!(closed.status, WorkflowV2Status::NeedsReview);
    assert_eq!(closed.attempt, left.attempt);
    assert_eq!(closed.result.data["inflight_marker"], false);
}

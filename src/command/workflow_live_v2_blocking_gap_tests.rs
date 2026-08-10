//! Issue #162: `events.jsonl` and `v2/results/` must agree about failure.
//!
//! A run writes itself down twice. `v2/results/` is authoritative but is only
//! readable by opening files; `events.jsonl` is the stream a monitor, the TUI,
//! or a human actually watches. On `wf-67dd2599-1463-499e-8622-3da72c13baba`
//! the two disagreed: the result store recorded a write-capable fanout that
//! ended `failed` with `severity: "blocking"` residual gaps naming a discarded
//! remediation wave, while the event stream named no gap at all. The run
//! presented as healthy for its whole duration.
//!
//! The test below is the issue's own verification procedure turned into an
//! assertion: drive a run that contains a blocking failure, then derive the
//! same two facts — which calls ended outside the accepted set, and which
//! blocking gaps were recorded — from each stream independently, and require
//! them to be equal. Asserting only "an event was emitted" would be weaker;
//! agreement between the two records is the invariant.

use super::*;

use std::collections::{BTreeMap, BTreeSet};

use archon_workflow::events::blocking_gap_events::{blocking_gap_ids, is_accepted_status};
use archon_workflow::v2::WorkflowV2CallRecord;
use archon_workflow::{WorkflowEvent, WorkflowEventKind};

/// Two write branches in one serial wave: one does its work, one returns a
/// safety failure. That is the shape from the issue — a single bad branch takes
/// the whole wave down — reproduced without needing a live agent.
#[derive(Default)]
struct BlockingWaveAgentClient {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for BlockingWaveAgentClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        Err(archon_workflow::WorkflowError::port(
            "the script is supplied directly; no planner call is expected",
        ))
    }

    async fn run_agent(
        &self,
        request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let addressed = serde_json::to_string(&request.messages).unwrap_or_default();
        let content = if addressed.contains(BAD_BRANCH) {
            failed_branch_output()
        } else {
            accepted_branch_output()
        };
        Ok(archon_workflow::WorkflowAgentOutcome {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

const WAVE_CALL: &str = "remediation-wave-1";
const GOOD_BRANCH: &str = "remediation-good";
const BAD_BRANCH: &str = "remediation-bad";

fn accepted_branch_output() -> String {
    serde_json::json!({
        "status": "accepted",
        "summary": "sibling remediation edited its declared target",
        "evidence": [
            {"kind": "implementation", "summary": "edited src/lib.rs, the declared target"}
        ],
        "files_changed": [{"path": "src/lib.rs", "purpose": "declared target edit"}],
        "task_coverage": [{
            "task_id": "TASK-EX-001",
            "status": "accepted",
            "summary": "sibling remediation landed",
            "evidence": [{"kind": "implementation", "summary": "src/lib.rs changed"}]
        }],
        "residual_gaps": [],
        "data": {"item_id": GOOD_BRANCH, "canonical_task_ids": ["TASK-EX-001"]}
    })
    .to_string()
}

/// A branch that returns a safety failure. `failure_kind: "safety"` is what the
/// write layer reads to decide the wave is terminally failed, and the blocking
/// residual gap is the thing that used to reach `v2/results/` and nothing else.
fn failed_branch_output() -> String {
    serde_json::json!({
        "status": "failed",
        "summary": "write branch declared no target ownership for the file it changed",
        "evidence": [
            {"kind": "blocker", "summary": "branch output was rejected before any patch was kept"}
        ],
        "residual_gaps": [{
            "id": "invalid_write_branch_output_remediation-bad",
            "description": "write branch 'remediation-bad' declares no target ownership; its work was discarded",
            "severity": "blocking"
        }],
        "data": {
            "item_id": BAD_BRANCH,
            "canonical_task_ids": ["TASK-EX-002"],
            "failure_kind": "safety",
            "branch_error_from_runtime": true
        }
    })
    .to_string()
}

const BLOCKING_WAVE_SCRIPT: &str = r#"
async function workflow(w) {
  await w.fanout("remediation-wave-1", [
    { id: "remediation-good", item_id: "remediation-good", canonical_task_ids: ["TASK-EX-001"], target_files: ["src/lib.rs"], task: "Remediate the first review finding." },
    { id: "remediation-bad", item_id: "remediation-bad", canonical_task_ids: ["TASK-EX-002"], target_files: ["src/main.rs"], task: "Remediate the second review finding." }
  ], { role: "coder", write: "serial", targetFilesFromItem: true, task: "Remediate one review finding." });
  await w.checkpoint("after-wave", { task: "Record that the script kept going past the failed wave." });
}
"#;

/// What a reader learns about failure from `v2/results/`.
#[derive(Debug, Default, PartialEq, Eq)]
struct FailureView {
    /// Call id -> status, for every call that ended outside the accepted set.
    non_accepted_calls: BTreeMap<String, String>,
    /// Every residual gap id recorded with `severity: "blocking"`.
    blocking_gap_ids: BTreeSet<String>,
}

fn failure_view_from_results(v2_store: &WorkflowV2ResultStore) -> FailureView {
    let mut view = FailureView::default();
    let dir = v2_store.root().join("results");
    let entries = std::fs::read_dir(&dir).expect("v2/results must exist after a run");
    for entry in entries {
        let path = entry.expect("results entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).expect("read result record");
        let Ok(record) = serde_json::from_str::<WorkflowV2CallRecord>(&raw) else {
            continue;
        };
        // Superseded archives sit beside the live record; only the live one is
        // the run's current statement about that call.
        if v2_store.result_path(&record.call.id) != path {
            continue;
        }
        if !is_accepted_status(record.status) {
            view.non_accepted_calls.insert(
                record.call.id.clone(),
                serde_json::to_value(record.status)
                    .expect("status is a string")
                    .as_str()
                    .expect("status is a string")
                    .to_string(),
            );
        }
        view.blocking_gap_ids
            .extend(blocking_gap_ids(&record.result));
    }
    view
}

/// The same two facts, derived only from the watchable stream.
///
/// "Non-accepted event" is read the way a monitor would read it: a kind that
/// means failure, or a `stage_completed` whose payload status is outside the
/// accepted set. Nothing here opens `v2/results/`.
fn failure_view_from_events(events_path: &std::path::Path) -> FailureView {
    let raw = std::fs::read_to_string(events_path).expect("events.jsonl must exist after a run");
    let mut view = FailureView::default();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let event: WorkflowEvent = serde_json::from_str(line).expect("event line parses");
        let status = event
            .detail
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let Some(call_id) = event
            .detail
            .get("call_id")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let reports_failure = match event.kind {
            WorkflowEventKind::StageFailed
            | WorkflowEventKind::StageStalled
            | WorkflowEventKind::BlockingGapDetected => true,
            WorkflowEventKind::StageCompleted => {
                !status.is_empty() && !matches!(status, "accepted" | "noop")
            }
            _ => false,
        };
        if reports_failure && !status.is_empty() && status != "accepted" && status != "noop" {
            view.non_accepted_calls
                .insert(call_id.to_string(), status.to_string());
        }
        if event.kind == WorkflowEventKind::BlockingGapDetected {
            let gap_id = event
                .detail
                .get("gap_id")
                .and_then(serde_json::Value::as_str)
                .expect("a blocking gap event must name its gap");
            assert!(
                !event
                    .detail
                    .get("gap_description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .is_empty(),
                "a blocking gap event must carry the description, so a reader \
                 never has to open v2/results/ to learn what blocked"
            );
            view.blocking_gap_ids.insert(gap_id.to_string());
        }
    }
    view
}

#[tokio::test]
async fn events_and_results_agree_on_a_run_containing_a_blocking_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(BlockingWaveAgentClient::default()),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "remediate two review findings".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    );

    let summary = runner
        .run(BLOCKING_WAVE_SCRIPT)
        .await
        .expect("script summary");
    assert_eq!(
        summary.status,
        WorkflowV2Status::Failed,
        "one safety-failed branch fails the wave, which fails the run"
    );

    let from_results = failure_view_from_results(&v2_store);
    let from_events = failure_view_from_events(&workflow_store.events_path(&run.id));

    // The run really did contain a blocking failure — otherwise agreement
    // between two empty views would prove nothing.
    assert_eq!(
        from_results
            .non_accepted_calls
            .get(WAVE_CALL)
            .map(String::as_str),
        Some("failed"),
        "v2/results must record the failed wave: {from_results:#?}"
    );
    assert!(
        from_results
            .blocking_gap_ids
            .iter()
            .any(|id| id.starts_with("write_fanout_failed_")),
        "v2/results must record the wave-level blocking gap: {from_results:#?}"
    );
    assert!(
        from_results
            .blocking_gap_ids
            .contains("invalid_write_branch_output_remediation-bad"),
        "v2/results must record the branch-level blocking gap: {from_results:#?}"
    );

    assert_eq!(
        from_events, from_results,
        "events.jsonl and v2/results must agree about which calls ended \
         outside the accepted set and which gaps blocked\n  events:  \
         {from_events:#?}\n  results: {from_results:#?}"
    );
}

/// The sibling branch's work is what the wave discarded, and the wave failure
/// says nothing about which sibling it was. This pins the per-branch record so
/// a later change to wave-vs-branch scoping cannot quietly stop recording it.
#[tokio::test]
async fn the_failed_wave_still_records_each_branch_outcome_separately() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(BlockingWaveAgentClient::default()),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "remediate two review findings".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );

    runner.run(BLOCKING_WAVE_SCRIPT).await.expect("summary");

    // Branch ids are the wave id joined to the item id, which is why the issue
    // quotes gaps like `invalid_write_branch_output_remediation-wave-1-...`.
    let outcomes = v2_store
        .load_branch_outcomes()
        .expect("load recorded branch outcomes");
    let by_item: BTreeMap<String, WorkflowV2Status> = outcomes
        .iter()
        .map(|outcome| (outcome.item_id.clone(), outcome.status))
        .collect();

    let good = by_item
        .iter()
        .find(|(item_id, _)| item_id.contains(GOOD_BRANCH))
        .map(|(_, status)| *status);
    assert_eq!(
        good,
        Some(WorkflowV2Status::Accepted),
        "the sibling did its work and its outcome must survive the wave failure: {by_item:#?}"
    );

    let bad = by_item
        .iter()
        .find(|(item_id, _)| item_id.contains(BAD_BRANCH))
        .map(|(_, status)| *status);
    assert_eq!(
        bad,
        Some(WorkflowV2Status::Failed),
        "the failed branch outcome must be recorded: {by_item:#?}"
    );
}

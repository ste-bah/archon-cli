//! Issue 337 round 4: a covered replay answers a call only while its record
//! is still an answer to it. A fault record (marked, or in an older binary's
//! shape) never replays, and a host command's answer about disk state replays
//! only while the disk still says the same; a crash such a record caused
//! heals on the resume after the fault or the disk is repaired.

use super::*;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;
use archon_workflow::{HostCommandRequest, HostCommandResult, WorkflowV2CallRecord};

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

fn outcome(stdout: &str, published: bool) -> HostCommandResult {
    HostCommandResult {
        exit_code: Some(0),
        stdout: stdout.into(),
        stderr: String::new(),
        stdout_bytes: stdout.len() as u64,
        stderr_bytes: 0,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: None,
        publication_receipt: published.then(|| archon_workflow::PublicationReceiptV1 {
            schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
            call_id: "host-command:probe:fixed".into(),
            command_id: "probe".into(),
            entries: Vec::new(),
            committed_at: "2026-10-06T00:00:00Z".into(),
        }),
        subjects: Vec::new(),
        postcondition: None,
    }
}

/// The first dispatch faults (`Io`); later ones answer "ok".
struct FaultOnce {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl WorkflowHostCommandExecutor for FaultOnce {
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
        _: HostCommandRequest,
        _: Option<u64>,
    ) -> archon_workflow::WorkflowResult<HostCommandResult> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(WorkflowError::Io {
                path: "/host/capture".into(),
                source: std::io::Error::other("disk full"),
            });
        }
        Ok(outcome("ok", false))
    }
}

/// Answers from the disk: "bad" until the operator repairs it, then "ok".
/// Its records are reusable (and live) only while the disk is unchanged.
struct DiskReader {
    repaired: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    published: bool,
}

#[async_trait::async_trait]
impl WorkflowHostCommandExecutor for DiskReader {
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
        Ok(!self.repaired.load(Ordering::SeqCst))
    }

    async fn execute(
        &self,
        _: HostCommandRequest,
        _: Option<u64>,
    ) -> archon_workflow::WorkflowResult<HostCommandResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = if self.repaired.load(Ordering::SeqCst) {
            "ok"
        } else {
            "bad"
        };
        Ok(outcome(answer, self.published))
    }
}

const ONE_CALL: &str = r#"async function workflow(w) {
  const gate = await w.hostCommand(args.command, { stdin: null });
  if (gate.stdout !== "ok") throw new Error("the host read failed: " + (gate.stdout || gate.summary));
  return gate.stdout;
}"#;

fn new_run(store: &WorkflowStore) -> String {
    store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "fixed-replay-guard".into(),
            task: "test covered replay guards".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap()
        .id
}

/// Runs the fixed script up to `max` times, resuming each pause; `between`
/// runs after each pause, before the resume.
async fn rounds(
    store: &WorkflowStore,
    run_id: &str,
    command: &str,
    executor: Arc<dyn WorkflowHostCommandExecutor>,
    max: usize,
    between: &dyn Fn(),
) -> Vec<RunStatus> {
    let mut statuses = Vec::new();
    for _ in 0..max {
        let current = store.load_state(run_id).unwrap();
        let plan = WorkflowScriptPlan::fixed(
            current.spec.clone(),
            ONE_CALL,
            Vec::new(),
            serde_json::json!({ "command": command }),
        );
        let (sink, _ui) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
        let _ = execute_fixed_decomposition_v2_run(
            store,
            current,
            plan,
            Arc::new(UnusedLlm),
            sink,
            Vec::new(),
            executor.clone(),
        )
        .await;
        let status = store.load_state(run_id).unwrap().status;
        statuses.push(status.clone());
        if status != RunStatus::Paused {
            break;
        }
        between();
        archon_workflow::LifecycleController::new(store.clone())
            .apply(run_id, archon_workflow::LifecycleAction::Resume)
            .unwrap();
    }
    statuses
}

fn v2_store(store: &WorkflowStore, run_id: &str) -> WorkflowV2ResultStore {
    WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
}

/// The slot record of the run's only host command.
fn host_record(store: &WorkflowStore, run_id: &str) -> WorkflowV2CallRecord {
    v2_store(store, run_id)
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| record.call.method == archon_workflow::WorkflowV2HostMethod::HostCommand)
        .expect("host command record")
}

fn assert_healed(statuses: &[RunStatus], calls: &AtomicUsize) {
    assert_eq!(
        statuses.len(),
        2,
        "one pause, then the run moves on: {statuses:?}"
    );
    assert_eq!(statuses[0], RunStatus::Paused);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "dispatched again: {statuses:?}"
    );
}

/// Review probe A: an older binary wrote the fault without the marker; a
/// pause on this binary snapshots it while it is still its slot's record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_old_binary_fault_record_never_replays_on_resume() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = new_run(&store);
    let calls = Arc::new(AtomicUsize::new(0));
    let statuses = rounds(
        &store,
        &run_id,
        "task-set-lint",
        Arc::new(FaultOnce {
            calls: calls.clone(),
        }),
        5,
        &|| {
            let mut record = host_record(&store, &run_id);
            if let Some(data) = record.result.data.as_object_mut() {
                data.remove(archon_workflow::v2::host_fault::HOST_DISPATCH_ERROR_MARKER);
            }
            let v2 = v2_store(&store, &run_id);
            v2.restore_call_record(&record).unwrap();
            super::super::workflow_live_v2_script::HostPauseCoverage::snapshot(&v2)
                .record(&store, &run_id, "probe", None);
        },
    )
    .await;
    assert_healed(&statuses, &calls);
}

/// A coverage record that names a fault record (as one written before the
/// classifier did) still never replays it: the replay checks again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_coverage_record_never_replays_the_fault_it_names() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = new_run(&store);
    let calls = Arc::new(AtomicUsize::new(0));
    let statuses = rounds(
        &store,
        &run_id,
        "task-set-lint",
        Arc::new(FaultOnce {
            calls: calls.clone(),
        }),
        5,
        &|| {
            let record = host_record(&store, &run_id);
            store
                .write_run_json(
                    &run_id,
                    "v2/script-pauses/host-legacy-g0.json",
                    &serde_json::json!({
                        "pause_id": "host-legacy-g0",
                        "joined": false,
                        "event_seq": null,
                        "generation": 1,
                        "host_taken": true,
                        "covered": [{
                            "call_id": record.call.id,
                            "attempt": record.attempt,
                            "input_hash": record.input_hash,
                        }],
                    }),
                )
                .unwrap();
        },
    )
    .await;
    assert_healed(&statuses, &calls);
}

/// Review probe B: a host read's unpublished answer about the disk made the
/// script throw; the operator repairs the disk and resumes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crash_a_disk_state_caused_heals_after_the_repair() {
    for (command, published) in [("verify-frozen-skeleton", false), ("land-task-body", true)] {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(temp.path().join("workflows"));
        let run_id = new_run(&store);
        let repaired = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let statuses = rounds(
            &store,
            &run_id,
            command,
            Arc::new(DiskReader {
                repaired: repaired.clone(),
                calls: calls.clone(),
                published,
            }),
            4,
            &|| repaired.store(true, Ordering::SeqCst),
        )
        .await;
        assert_healed(&statuses, &calls);
    }
}

pub(super) use std::collections::BTreeMap;
pub(super) use std::sync::Arc;
pub(super) use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) use crate::command::tui_workflow_ui_sink::default_workflow_ui_sink;
pub(super) use archon_workflow::{RunStatus, StageStatus, WorkflowSpec, WorkflowV2TaskCoverage};
pub(super) use archon_workflow::{WorkflowAgentOutcome, WorkflowLlmClient};

use super::*;

#[tokio::test]
async fn closed_tui_prevents_script_host_call_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, tui_rx) = default_workflow_ui_sink();
    drop(tui_rx);
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "closed tui".to_string(),
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

    let error = runner
        .run(
            r#"
	async function workflow(w) {
	  await w.checkpoint("must-not-run");
	}
	"#,
        )
        .await
        .expect_err("closed TUI must reject the script call");

    assert!(matches!(error, WorkflowError::NotificationDelivery(_)));
    assert!(
        v2_store
            .load_call_record("must-not-run")
            .expect("call record lookup")
            .is_none(),
        "host call persisted after status delivery failed"
    );
}

#[test]
fn script_source_injects_saved_workflow_args() {
    let source = script_source(
        "export default async function workflow(w) { return args.issue; }",
        Some(&serde_json::json!({ "issue": 1024, "labels": ["bug"] })),
    );

    assert!(source.contains("globalThis.args = {"));
    assert!(source.contains(r#""issue":1024"#));
    assert!(source.contains(r#""labels":["bug"]"#));
}

#[test]
fn script_source_omits_args_as_undefined() {
    let source = script_source(
        "export default async function workflow(w) { return typeof args; }",
        None,
    );

    assert!(source.contains("globalThis.args = undefined;"));
}

#[tokio::test]
async fn human_gate_stops_script_before_later_calls() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "needs confirmation".to_string(),
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

    let summary = runner
        .run(
            r#"
	async function workflow(w) {
	  const gate = await w.humanGate("confirm-before-write", { task: "Confirm before writing" });
	  await w.checkpoint("should-not-run");
	}
	"#,
        )
        .await
        .expect("script summary");

    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert_eq!(summary.executed, 1);
    assert_eq!(summary.completed, 0);
    assert_eq!(summary.failed_call.as_deref(), Some("confirm-before-write"));
    assert!(
        v2_store
            .load_call_record("confirm-before-write")
            .expect("confirm record")
            .is_some()
    );
    assert!(
        v2_store
            .load_call_record("should-not-run")
            .expect("checkpoint lookup")
            .is_none()
    );
}

#[tokio::test]
async fn failed_reduce_returns_error_value_for_script_owned_remediation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(AlwaysInvalidLlm {
            calls: AtomicUsize::new(0),
        }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "reduce must fail terminally".to_string(),
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

    let summary = runner
            .run(
                r#"
	async function workflow(w) {
	  const reduced = await w.reduce("reduce-discovery", [{ id: "a", summary: "large branch data" }], { role: "reducer", task: "Reduce discovery into implementation inventory" });
	  if (reduced.status === "failed") {
	    await w.checkpoint("script-owned-remediation");
	  }
	  await w.checkpoint("script-continued");
	}
	"#,
            )
            .await
            .expect("script summary");

    // Errors are values: the failed reduce flows back to the script, the
    // script-owned remediation branch is reachable, and the run continues.
    assert_eq!(summary.status, WorkflowV2Status::Failed);
    assert_eq!(summary.executed, 3);
    assert!(
        v2_store
            .load_call_record("reduce-discovery")
            .expect("reduce lookup")
            .is_some_and(|record| record.status == WorkflowV2Status::Failed)
    );
    assert!(
        v2_store
            .load_call_record("script-owned-remediation")
            .expect("remediation branch lookup")
            .is_some()
    );
    assert!(
        v2_store
            .load_call_record("script-continued")
            .expect("checkpoint lookup")
            .is_some()
    );
}

#[tokio::test]
async fn generated_inventory_schema_failure_returns_script_repair_data() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(AlwaysInvalidLlm {
            calls: AtomicUsize::new(0),
        }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "generated decomposed PRD inventory repair".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        Some(task_universe()),
        None,
    );

    let summary = runner
            .run(
                r#"
async function workflow(w) {
  const inventory = await w.reduce("canonical-implementation-inventory", [], {
    tier: "reducer",
    task: "Produce canonical generated PRD inventory."
  });
  if (inventory.status !== "needs_review") {
    await w.checkpoint("bad-inventory-status");
  }
  if (!inventory.unresolved_issues || inventory.unresolved_issues[0].kind !== "inventory_shape_repair") {
    await w.checkpoint("missing-inventory-repair-issue");
  }
  await w.checkpoint("script-owned-inventory-repair-visible");
}
"#,
            )
            .await
            .expect("script summary");

    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert_eq!(summary.executed, 2);
    assert_eq!(summary.completed, 1);
    assert!(summary.failed_call.is_none());
    assert!(
        v2_store
            .load_call_record("canonical-implementation-inventory")
            .expect("inventory record")
            .is_some_and(|record| record.status == WorkflowV2Status::NeedsReview)
    );
    assert!(
        v2_store
            .load_call_record("script-owned-inventory-repair-visible")
            .expect("repair checkpoint")
            .is_some()
    );
    assert!(
        v2_store
            .load_call_record("bad-inventory-status")
            .expect("bad status lookup")
            .is_none()
    );
    assert!(
        v2_store
            .load_call_record("missing-inventory-repair-issue")
            .expect("missing issue lookup")
            .is_none()
    );
}

#[tokio::test]
async fn non_accepted_quality_gate_returns_value_the_script_consumes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "quality gate must stop terminally".to_string(),
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

    let summary = runner
            .run(
                r#"
async function workflow(w) {
  const gate = await w.qualityGate("quality", { task: "Check typed inputs before final report" });
  await w.finalReport("final", { status: gate.status, inputs: [gate], task: "Report the gate outcome either way" });
}
"#,
            )
            .await
            .expect("script summary");

    // Errors are values: the non-accepted gate flows to the script, which
    // still produces a final report describing the outcome.
    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert_eq!(summary.failed_call.as_deref(), Some("final"));
    assert!(
        v2_store
            .load_call_record("quality")
            .expect("quality lookup")
            .is_some_and(|record| record.status == WorkflowV2Status::NeedsReview)
    );
    assert!(
        v2_store
            .load_call_record("final")
            .expect("final lookup")
            .is_some()
    );
}

#[tokio::test]
async fn non_accepted_final_report_still_ends_the_script() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "generated decomposed PRD".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        Some(task_universe()),
        None,
    );

    let summary = runner
            .run(
                r#"
async function workflow(w) {
  await w.finalReport("blocked-report", { status: "needs_review", inputs: {}, task: "Stop with review data" });
  await w.checkpoint("should-not-run-after-final-report");
}
"#,
            )
            .await
            .expect("script summary");

    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert_eq!(summary.failed_call.as_deref(), Some("blocked-report"));
    assert!(
        v2_store
            .load_call_record("should-not-run-after-final-report")
            .expect("checkpoint lookup")
            .is_none()
    );
}

#[tokio::test]
async fn explicit_source_argument_wins_over_options_inputs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "explicit source must drive final gates".to_string(),
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

    let summary = runner
        .run(
            r#"
async function workflow(w) {
  await w.qualityGate(
    "source-gate",
    [{ status: "accepted", summary: "explicit source accepted" }],
    { inputs: [{ status: "failed", summary: "options.inputs must not replace source" }] }
  );
  await w.checkpoint("after-source-gate");
}
"#,
        )
        .await
        .expect("script summary");

    assert_eq!(summary.status, WorkflowV2Status::Accepted);
    assert_eq!(summary.executed, 2);
    assert!(
        v2_store
            .load_call_record("source-gate")
            .expect("quality lookup")
            .is_some_and(|record| record.status == WorkflowV2Status::Accepted)
    );
    assert!(
        v2_store
            .load_call_record("after-source-gate")
            .expect("checkpoint lookup")
            .is_some()
    );
}

struct FakeHostCommandExecutor {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor
    for FakeHostCommandExecutor
{
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}:fixed", request.command_id))
    }

    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(0),
            stdout: format!("{} passed", request.command_id),
            stderr: String::new(),
            stdout_bytes: 20,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: None,
            publication_receipt: None,
        })
    }
}

#[tokio::test]
async fn host_command_uses_persisted_call_record_and_checkpoint_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let executor = Arc::new(FakeHostCommandExecutor {
        calls: AtomicUsize::new(0),
    });
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "host command persistence".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    )
    .with_host_command_executor(executor.clone());

    let summary = runner
        .run(
            r#"
async function workflow(w) {
  const result = await w.hostCommand("task-set-lint", { stdin: null });
  if (result.exitCode !== 0 || !result.stdout.includes("passed")) {
    throw new Error("typed host command result missing");
  }
  return result;
}
"#,
        )
        .await
        .expect("host command script");

    let call_id = "host-command:task-set-lint:fixed";
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(summary.executed, 1);
    let record = v2_store
        .load_call_record(call_id)
        .expect("record lookup")
        .expect("persisted host command record");
    assert_eq!(record.call.method, WorkflowV2HostMethod::HostCommand);
    assert_eq!(record.status, WorkflowV2Status::Accepted);
    assert_eq!(record.result.data["exitCode"], 0);
    assert!(
        v2_store
            .load_call_record("hostCommand#1")
            .expect("transport id lookup")
            .is_none(),
        "transport correlation id must not become reuse identity"
    );
    let checkpoint = v2_store
        .load_checkpoint()
        .expect("checkpoint lookup")
        .expect("checkpoint");
    assert!(checkpoint.completed_call_ids.contains(&call_id.to_string()));
}

#[tokio::test]
async fn host_command_is_reused_without_second_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let executor = Arc::new(FakeHostCommandExecutor {
        calls: AtomicUsize::new(0),
    });
    let script = r#"async function workflow(w) { return await w.hostCommand("task-set-lint", { stdin: null }); }"#;

    for _ in 0..2 {
        let client = LiveV2AgentClient::new(
            Arc::new(PanicLlm),
            ui_sink.clone(),
            Vec::new(),
            run.id.clone(),
            None,
            None,
        );
        WorkflowV2ScriptRunner::new(
            "host command reuse".to_string(),
            test_runtime(&spec),
            WorkflowV2AgentAdapter::new(),
            client,
            v2_store.clone(),
            workflow_store.clone(),
            run.id.clone(),
            true,
            None,
            None,
        )
        .with_host_command_executor(executor.clone())
        .run(script)
        .await
        .expect("host command run");
    }

    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
}

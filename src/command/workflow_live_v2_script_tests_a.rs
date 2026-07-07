    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use anyhow::Result;
    use archon_pipeline::runner::{LlmClient, LlmResponse};
    use archon_tui::event_channel::bounded_tui_event_channel;
    use archon_workflow::{
        RunStatus, StageStatus, WorkflowSpec, WorkflowV2Evidence, WorkflowV2EvidenceKind,
        WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
    };

    use super::*;

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
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client = LiveV2AgentClient::new(
            Arc::new(PanicLlm),
            tui_tx,
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
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client = LiveV2AgentClient::new(
            Arc::new(AlwaysInvalidLlm {
                calls: AtomicUsize::new(0),
            }),
            tui_tx,
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
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client = LiveV2AgentClient::new(
            Arc::new(AlwaysInvalidLlm {
                calls: AtomicUsize::new(0),
            }),
            tui_tx,
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
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client = LiveV2AgentClient::new(
            Arc::new(PanicLlm),
            tui_tx,
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
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client = LiveV2AgentClient::new(
            Arc::new(PanicLlm),
            tui_tx,
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
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client = LiveV2AgentClient::new(
            Arc::new(PanicLlm),
            tui_tx,
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

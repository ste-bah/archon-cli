    #[test]
    fn status_merge_preserves_terminal_or_review_state() {
        assert_eq!(
            merge_v2_status(WorkflowV2Status::Failed, WorkflowV2Status::Accepted),
            WorkflowV2Status::Failed
        );
        assert_eq!(
            merge_v2_status(WorkflowV2Status::NeedsReview, WorkflowV2Status::Accepted),
            WorkflowV2Status::NeedsReview
        );
        assert_eq!(
            merge_v2_status(WorkflowV2Status::Accepted, WorkflowV2Status::Cancelled),
            WorkflowV2Status::Cancelled
        );
    }

    #[test]
    fn read_only_calls_cannot_accept_task_coverage_without_implementation_evidence() {
        let mut result = WorkflowV2Result::accepted("read-only audit claimed completion");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "read task files",
        ));
        result.task_coverage.push(WorkflowV2TaskCoverage {
            task_id: "TASK-TDL-001".to_string(),
            status: WorkflowV2TaskCoverageStatus::Accepted,
            summary: "claimed done".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "inspected requirements",
            )],
        });
        let execution = WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "readonly-audit".to_string(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: WorkflowV2HostOptions::default(),
            },
            input: serde_json::Value::Null,
            depends_on: Vec::new(),
        };

        let result = normalize_result_for_call(&execution, result);

        assert_eq!(result.status, WorkflowV2Status::NeedsReview);
        assert_eq!(
            result.task_coverage[0].status,
            WorkflowV2TaskCoverageStatus::Unknown
        );
        assert!(
            result
                .residual_gaps
                .iter()
                .any(|gap| gap.id == "read_only_task_acceptance_readonly-audit")
        );
    }

    #[test]
    fn artifact_only_focused_verification_can_accept_verification_coverage() {
        let result: WorkflowV2Result = serde_json::from_str(include_str!(
            "fixtures/wf32_artifact_verification_aggregate.json"
        ))
        .expect("D16 fixture");
        let options = WorkflowV2HostOptions {
            item_kind: Some("focused_verification".to_string()),
            ..WorkflowV2HostOptions::default()
        };
        let execution = WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "verification-wave-fixture".to_string(),
                method: WorkflowV2HostMethod::Parallel,
                write_mode: None,
                options,
            },
            input: serde_json::Value::Null,
            depends_on: Vec::new(),
        };

        let result = normalize_result_for_call(&execution, result);

        assert_eq!(result.status, WorkflowV2Status::Accepted);
        assert_eq!(
            result.task_coverage[0].status,
            WorkflowV2TaskCoverageStatus::Accepted
        );
        assert!(!result
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with("read_only_task_acceptance_")));
    }

    #[test]
    fn empty_items_output_requires_noop_proof() {
        let mut result = WorkflowV2Result::accepted("inventory complete");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "read all tasks",
        ));
        result.data = serde_json::json!({ "items": [] });
        let mut options = WorkflowV2HostOptions::default();
        options
            .extra
            .insert("outputs".to_string(), serde_json::json!(["items"]));
        let execution = WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "implementation-inventory".to_string(),
                method: WorkflowV2HostMethod::Reduce,
                write_mode: None,
                options,
            },
            input: serde_json::Value::Null,
            depends_on: Vec::new(),
        };

        let result = normalize_result_for_call(&execution, result);

        assert_eq!(result.status, WorkflowV2Status::NeedsReview);
        assert!(
            result
                .residual_gaps
                .iter()
                .any(|gap| gap.id == "empty_items_output_implementation-inventory")
        );
    }

    #[tokio::test]
    async fn dynamic_loop_host_calls_are_recorded_with_runtime_ids() {
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
            "dynamic loop checkpoints".to_string(),
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
            .run(
                r#"
async function workflow(w) {
  let iteration = 1;
  while (iteration <= 2) {
    await w.checkpoint("loop-checkpoint-" + iteration, { iteration });
    iteration += 1;
  }
}
"#,
            )
            .await
            .expect("script summary");

        assert_eq!(summary.status, WorkflowV2Status::Accepted);
        assert_eq!(summary.executed, 2);
        assert_eq!(summary.completed, 2);
        assert_eq!(
            summary
                .calls
                .iter()
                .map(|call| call.id.as_str())
                .collect::<Vec<_>>(),
            vec!["loop-checkpoint-1", "loop-checkpoint-2"]
        );
        assert!(
            v2_store
                .load_call_record("loop-checkpoint-1")
                .expect("first checkpoint lookup")
                .is_some()
        );
        assert!(
            v2_store
                .load_call_record("loop-checkpoint-2")
                .expect("second checkpoint lookup")
                .is_some()
        );
        let run = workflow_store.load_state(&run.id).expect("run state");
        assert_eq!(
            run.stages
                .get("loop-checkpoint-1")
                .expect("first runtime stage")
                .status,
            StageStatus::Running
        );
        assert_eq!(
            run.stages
                .get("loop-checkpoint-2")
                .expect("second runtime stage")
                .status,
            StageStatus::Running
        );
    }

// The scaffold is a native Rust lifecycle with a descriptor record; the
// declared static plan is its single source of truth for approval metadata.
#[test]
fn static_scaffold_plan_has_unique_ids_and_write_isolated_fanouts() {
    use std::collections::BTreeSet;
    let plan = super::super::super::workflow_live_generated_scaffold::decomposed_prd_plan_calls();
    assert!(!plan.is_empty());
    let mut seen = BTreeSet::new();
    for call in &plan {
        assert!(seen.insert(call.id.clone()), "duplicate id {}", call.id);
        if call.method == WorkflowV2HostMethod::Fanout {
            assert_eq!(
                call.write_mode,
                Some(WorkflowV2WriteMode::Worktree),
                "{}",
                call.id
            );
        }
    }
}

#[tokio::test]
async fn dry_run_rejects_provider_routing_and_duplicate_ids() {
    let duplicate = r#"
async function workflow(w) {
  await w.checkpoint("same-id");
  await w.checkpoint("same-id");
}
"#;
    let error = dry_run_workflow_plan(duplicate, None)
        .await
        .expect_err("duplicate ids must fail");
    assert!(
        error.to_string().contains("duplicate host call id"),
        "{error}"
    );

    let routed = r#"
async function workflow(w) {
  await w.agent("routed", { model: "claude-opus-4-8", task: "inspect" });
}
"#;
    let error = dry_run_workflow_plan(routed, None)
        .await
        .expect_err("model override must fail");
    assert!(
        error.to_string().contains("provider routing is host policy"),
        "{error}"
    );

    let syntax_error = r#"
async function workflow(w) {
  await w.agent("broken", { task: "inspect" }
}
"#;
    let error = dry_run_workflow_plan(syntax_error, None)
        .await
        .expect_err("syntax error must fail");
    assert!(error.to_string().contains("validation failed"), "{error}");
}

#[tokio::test]
async fn determinism_prelude_blocks_wall_clock_and_randomness() {
    for forbidden in ["Math.random()", "Date.now()", "new Date()"] {
        let source = format!(
            r#"
async function workflow(w) {{
  const value = {forbidden};
  await w.checkpoint("uses-nondeterminism-" + value);
}}
"#
        );
        let error = dry_run_workflow_plan(&source, None)
            .await
            .expect_err("nondeterministic script must fail");
        assert!(
            error.to_string().contains("unavailable in workflow scripts"),
            "{forbidden}: {error}"
        );
    }
}

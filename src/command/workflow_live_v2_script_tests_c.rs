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

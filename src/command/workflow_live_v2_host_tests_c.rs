#[test]
fn missing_required_artifact_becomes_review_choice_not_blocked_engine_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let missing = temp.path().join("missing.json");
    let require = WorkflowV2CallExecution {
        input: serde_json::json!({ "path": missing }),
        ..execution(
            "require-artifact",
            WorkflowV2HostMethod::RequireArtifact,
            None,
        )
    };

    let result = execute_local_host_call(&require, &store, None)
        .expect("require")
        .expect("local result");

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        result.residual_gaps[0].severity.as_deref(),
        Some("remediation")
    );
    assert!(
        result
            .data
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|choices| !choices.is_empty())
    );
}

#[test]
fn quality_gate_returns_review_choices_for_non_accepted_inputs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let upstream = execution("audit", WorkflowV2HostMethod::Agent, None);
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "audit found gaps".to_string(),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "review evidence",
    ));
    store
        .save_call_record(&archon_workflow::WorkflowV2CallRecord::new(
            store.run_id(),
            upstream.call,
            1,
            "hash".to_string(),
            result,
            Vec::new(),
        ))
        .expect("record");

    let gate = execution(
        "quality",
        WorkflowV2HostMethod::QualityGate,
        Some("[audit]"),
    );
    let result = execute_local_host_call(&gate, &store, None)
        .expect("quality")
        .expect("local result");

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.residual_gaps[0].severity.as_deref(), Some("review"));
    assert!(
        result
            .data
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|choices| choices.iter().any(|choice| choice
                .get("id")
                .and_then(serde_json::Value::as_str)
                == Some("run_remediation")))
    );
}

#[test]
fn human_gate_is_never_auto_accepted_and_returns_choices() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let gate = execution("approval", WorkflowV2HostMethod::HumanGate, None);

    let result = execute_local_host_call(&gate, &store, None)
        .expect("gate")
        .expect("local result");

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(!result.residual_gaps.is_empty());
    assert!(
        result
            .data
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|choices| choices.iter().any(|choice| choice
                .get("id")
                .and_then(serde_json::Value::as_str)
                == Some("approve_continue")))
    );
}

fn execution(
    id: &str,
    method: WorkflowV2HostMethod,
    source: Option<&str>,
) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: id.to_string(),
            method,
            write_mode: None,
            options: WorkflowV2HostOptions {
                source: source.map(str::to_string),
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({ "payload": id }),
        depends_on: Vec::new(),
    }
}

fn task_universe_080() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-080".to_string(),
            aliases: vec!["T080".to_string()],
            source_path: "/tmp/tasks/TASK-TDL-080.md".to_string(),
            dependency_ids: Vec::new(),
            title: None,
            acceptance_criteria: vec!["Coverage proof is current and complete.".to_string()],
            artifact_requirements: Vec::new(),
            ..Default::default()
        }],
    }
}

fn task_universe_010() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-010".to_string(),
            aliases: vec!["T010".to_string()],
            source_path: "/tmp/tasks/TASK-TDL-010.md".to_string(),
            dependency_ids: Vec::new(),
            title: None,
            acceptance_criteria: vec!["Registry schema proof is current and complete.".to_string()],
            artifact_requirements: Vec::new(),
            ..Default::default()
        }],
    }
}

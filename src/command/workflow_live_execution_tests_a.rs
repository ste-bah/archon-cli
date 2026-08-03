use super::*;

#[tokio::test]
async fn live_planner_validation_failure_does_not_fallback_to_smoke_plan() {
    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::bounded_workflow_ui_sink(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let err = plan_live(
        &store,
        "implement the whole PRD",
        Arc::new(InvalidPlanner),
        ui_sink,
        &default_generated_workflow_config(),
        &archon_core::config::LearningConfig::default(),
    )
    .await
    .expect_err("invalid live plans must fail instead of using heuristic fallback");
    assert!(err.to_string().contains("planner failure recorded at"));
    let failure_dir = store.root().join("planner-failures");
    let entries = std::fs::read_dir(&failure_dir)
        .expect("planner failure directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("planner failure entries");
    assert_eq!(entries.len(), 1);
    let body = std::fs::read_to_string(entries[0].path()).expect("planner failure body");
    let value: serde_json::Value = serde_json::from_str(&body).expect("planner failure json");
    assert!(
        value["attempts"]
            .as_array()
            .is_some_and(|attempts| !attempts.is_empty())
    );
    assert!(value["attempts"][0]["content_hash"].as_str().is_some());
    assert!(value["attempts"][0]["content"].as_str().is_some());
    assert!(value["attempts"][0]["content_preview"].as_str().is_some());
}

#[tokio::test]
async fn transient_planner_repair_retry_aborts_when_notification_is_rejected() {
    let planner = Arc::new(PlannerRepairRetryClient {
        calls: AtomicUsize::new(0),
        repair_started: tokio::sync::Notify::new(),
        release_repair: tokio::sync::Notify::new(),
    });
    let (ui_sink, mut tui_rx) = crate::command::tui_workflow_ui_sink::bounded_workflow_ui_sink(8);
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    let run_planner = planner.clone();
    let handle = tokio::spawn(async move {
        plan_live(
            &store,
            "Inspect the repository",
            run_planner,
            ui_sink,
            &default_generated_workflow_config(),
            &archon_core::config::LearningConfig::default(),
        )
        .await
    });

    planner.repair_started.notified().await;
    while tui_rx.try_recv().is_ok() {}
    drop(tui_rx);
    planner.release_repair.notify_one();

    handle
        .await
        .expect("planner join")
        .expect_err("rejected retry notification must abort planner repair retry");
    assert_eq!(planner.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn transient_planner_retry_aborts_when_notification_is_rejected() {
    let planner = Arc::new(FlakyPlanner {
        calls: AtomicUsize::new(0),
        first_error: "LLM stream error (server_error): temporary upstream failure",
    });

    super::super::workflow_live_retry::send_message_with_transient_retry(
        &(planner.clone() as Arc<dyn WorkflowLlmClient>),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        "sonnet",
        |_| async { anyhow::bail!("retry notification rejected") },
    )
    .await
    .expect_err("rejected notification must prevent transient planner retry");

    assert_eq!(planner.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn live_planner_retries_transient_stream_server_errors() {
    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::bounded_workflow_ui_sink(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let planner = Arc::new(FlakyPlanner {
        calls: AtomicUsize::new(0),
        first_error: "LLM stream error (server_error): temporary upstream failure",
    });

    let plan = plan_live(
        &store,
        "inspect the repository",
        planner.clone(),
        ui_sink,
        &default_generated_workflow_config(),
        &archon_core::config::LearningConfig::default(),
    )
    .await
    .expect("transient planner stream failure should retry and recover");

    assert_eq!(plan.calls.len(), 1);
    assert_eq!(plan.calls[0].id, "discover");
    assert_eq!(planner.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn implementation_prd_plan_uses_deterministic_scaffold_not_provider_fanout() {
    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::bounded_workflow_ui_sink(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let tasks = temp.path().join("tasks/PRD-EXAMPLE-001");
    std::fs::create_dir_all(&tasks).expect("task dir");
    std::fs::write(
        tasks.join("TASK-TDL-001-foundation.md"),
        standard_task_file("TASK-TDL-001", "[]", "['TASK-TDL-010']", ""),
    )
    .expect("task 1");
    std::fs::write(
        tasks.join("TASK-TDL-010-dependent.md"),
        standard_task_file("TASK-TDL-010", "['TASK-TDL-001']", "[]", ""),
    )
    .expect("task 10");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let planner = Arc::new(GuttedImplementationPlanner {
        calls: AtomicUsize::new(0),
    });

    let plan = plan_live(
        &store,
        &format!(
            "Implement the decomposed PRD at {} against the repository {}",
            tasks.display(),
            temp.path().display()
        ),
        planner.clone(),
        ui_sink,
        &default_generated_workflow_config(),
        &archon_core::config::LearningConfig::default(),
    )
    .await
    .expect("decomposed PRD planning should use the deterministic scaffold");

    assert_eq!(planner.calls.load(Ordering::SeqCst), 0);
    assert!(
        plan.harness_source
            .contains("# Archon decomposed-PRD workflow (native lifecycle v1)"),
        "{}",
        plan.harness_source
    );
    assert!(
        plan.harness_source.contains("task_universe:"),
        "{}",
        plan.harness_source
    );
    assert!(
        plan.harness_source
            .contains("- implementation-wave (fanout, write=Worktree)"),
        "{}",
        plan.harness_source
    );
    assert!(
        plan.harness_source
            .contains("- verification-wave (parallel)"),
        "{}",
        plan.harness_source
    );
    assert!(
        plan.harness_source
            .contains("- remediation-wave (fanout, write=Worktree)"),
        "{}",
        plan.harness_source
    );
    assert!(!plan.harness_source.contains("w.humanGate"));
    assert!(!plan.harness_source.contains("w.implementation("));
    let implementation_call = plan
        .calls
        .iter()
        .find(|call| {
            call.id == "implementation-wave"
                && call
                    .options
                    .extra
                    .get("dynamic_id_prefix")
                    .and_then(serde_json::Value::as_str)
                    == Some("implementation-wave-")
        })
        .expect("implementation wave call");
    assert_eq!(
        implementation_call.options.source.as_deref(),
        Some("readyImplementationItems")
    );
    assert!(
        format!("{:?}", implementation_call.write_mode).contains("Worktree"),
        "{implementation_call:?}"
    );
    let scaffold = plan
        .generated_scaffold()
        .expect("decomposed PRD plan exposes generated scaffold metadata");
    assert_eq!(
        scaffold.kind,
        GeneratedWorkflowKind::DecomposedPrdScriptScaffold
    );
    assert_eq!(
        scaffold.scaffold_hash,
        workflow_scaffold_hash(&plan.harness_source)
    );
    assert_eq!(scaffold.host_call_manifest.len(), plan.calls.len());
    assert_unique_host_call_ids(&plan.calls);
    assert_unique_host_call_ids(&scaffold.host_call_manifest);
    assert!(
        scaffold.prompt_slots.contains_key("implementation_wave"),
        "{:?}",
        scaffold.prompt_slots
    );
    assert_eq!(scaffold.task_universe["tasks"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn implementation_prd_plan_embeds_governed_learning_context_from_prior_runs() {
    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::bounded_workflow_ui_sink(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let tasks = temp.path().join("tasks/PRD-EXAMPLE-001");
    std::fs::create_dir_all(&tasks).expect("task dir");
    std::fs::write(
        tasks.join("TASK-TDL-001-foundation.md"),
        standard_task_file("TASK-TDL-001", "[]", "[]", ""),
    )
    .expect("task 1");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let prior_run = store
        .create_run(WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "prior-generated".to_string(),
            task: "Prior generated workflow".to_string(),
            target_repository_root: None,
            max_parallelism: 4,
            max_agents: 16,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .expect("prior run");
    let learning_dir = store.run_dir(&prior_run.id).join("learning");
    std::fs::create_dir_all(&learning_dir).expect("learning dir");
    let learning_event = WorkflowLearningEvent::generated_run(
        prior_run.id.clone(),
        "prior-scaffold-hash",
        WorkflowV2Status::NeedsReview,
        Some("final_evidence_gap".to_string()),
        true,
        vec![WorkflowLearningEvidenceRef::call(
            "failed_call",
            "final-zero-gap-audit",
        )],
    );
    std::fs::write(
        learning_dir.join("generated-workflow-events.jsonl"),
        format!(
            "{}\n",
            serde_json::to_string(&learning_event).expect("learning json")
        ),
    )
    .expect("learning event");
    let planner = Arc::new(GuttedImplementationPlanner {
        calls: AtomicUsize::new(0),
    });

    let plan = plan_live(
        &store,
        &format!(
            "Implement the decomposed PRD at {} against the repository {}",
            tasks.display(),
            temp.path().display()
        ),
        planner,
        ui_sink,
        &default_generated_workflow_config(),
        &archon_core::config::LearningConfig::default(),
    )
    .await
    .expect("decomposed PRD planning should use deterministic scaffold");

    assert!(plan.harness_source.contains("governed_learning_context:"));
    assert!(plan.harness_source.contains("final_evidence_gap"));
    assert_eq!(plan.governed_learning_context.len(), 1);
    let scaffold = plan
        .generated_scaffold()
        .expect("generated scaffold metadata");
    assert_eq!(scaffold.governed_learning_context.len(), 1);
    assert_eq!(
        scaffold.governed_learning_context[0].source_run_id,
        prior_run.id
    );
}

fn assert_unique_host_call_ids(calls: &[WorkflowV2HostCall]) {
    let mut seen = BTreeSet::new();
    for call in calls {
        assert!(
            seen.insert(call.id.as_str()),
            "duplicate generated host call id {}",
            call.id
        );
    }
}

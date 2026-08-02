#[tokio::test]
async fn generated_workflow_ignores_legacy_hash_only_deny_for_new_approval_subject() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let tasks = temp.path().join("tasks/PRD-EXAMPLE-001");
    std::fs::create_dir_all(&tasks).expect("task dir");
    std::fs::write(
        tasks.join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: []\n",
    )
    .expect("task 1");
    std::fs::write(
        tasks.join("TASK-TDL-010-dependent.md"),
        "# Dependent\n\ntask_id: TASK-TDL-010\ndepends_on: ['TASK-TDL-001']\n",
    )
    .expect("task 10");
    let task = format!(
        "Implement the decomposed PRD at {} against the repository {}",
        tasks.display(),
        temp.path().display()
    );
    let store = WorkflowStore::project(temp.path());
    let planner = Arc::new(GuttedImplementationPlanner {
        calls: AtomicUsize::new(0),
    });
    let seed_plan = plan_live(
        &store,
        &task,
        planner.clone(),
        tui_tx.clone(),
        &default_generated_workflow_config(),
    )
    .await
    .expect("seed deterministic generated plan");
    let seed_run = store
        .create_run(seed_plan.approval_metadata_spec())
        .expect("seed generated run");
    let seed_manifest = WorkflowBundle::create_for_run(
        &store,
        &seed_run,
        &seed_plan.harness_source,
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .expect("seed bundle")
    .manifest;
    let approvals = WorkflowApprovalStore::project(temp.path());
    std::fs::create_dir_all(approvals.path().parent().expect("approvals parent"))
        .expect("approval dir");
    let project_root = temp
        .path()
        .canonicalize()
        .unwrap_or_else(|_| temp.path().to_path_buf())
        .display()
        .to_string();
    std::fs::write(
        approvals.path(),
        serde_json::to_vec_pretty(&json!({
            "records": [{
                "workflow_hash": seed_manifest.workflow_hash.clone(),
                "project_root": project_root,
                "workflow_name": seed_manifest.name.clone(),
                "decision": "denied",
                "decided_at": "2026-01-01T00:00:00Z",
                "decided_by": "legacy-test",
                "run_id": seed_run.id.clone(),
                "phase_count": seed_manifest.phase_count,
                "max_agents": seed_manifest.max_agents,
                "max_parallelism": seed_manifest.max_parallelism,
                "write_capable_stages": seed_manifest.write_capable_stages.clone(),
                "external_requirements": [],
                "raw_script_path": store.run_dir(&seed_run.id).join("workflow.js").display().to_string(),
                "compiled_spec_path": store.run_dir(&seed_run.id).join("workflow.compiled.yaml").display().to_string(),
                "origin": "generated_harness"
            }]
        }))
        .expect("legacy approval json"),
    )
    .expect("seed legacy approval");

    let output = run_live_action(
        temp.path(),
        CommandAction::Run { task, decomposed: false },
        planner,
        tui_tx,
        None,
        default_generated_workflow_config(),
        true,
        LiveApprovalMode::InteractiveSurface,
    )
    .await
    .expect("generated workflow should reach approval gate");

    assert!(output.contains("Workflow awaiting approval:"), "{output}");
    assert!(output.contains("Approval subject:"), "{output}");
    assert!(output.contains("generated_metadata="), "{output}");
    assert!(
        !output.contains("Workflow denied and cancelled"),
        "{output}"
    );
    let run_id = output
        .lines()
        .find_map(|line| line.strip_prefix("Workflow awaiting approval: "))
        .expect("pending run id")
        .trim();
    let run = store.load_state(run_id).expect("pending generated run");
    assert_eq!(run.status, RunStatus::Paused);
    let inspection = approvals
        .inspect_run(temp.path(), &store, &run)
        .expect("approval inspection");
    assert!(inspection.decision.is_none());
    assert_ne!(inspection.workflow_hash, seed_manifest.workflow_hash);
    assert!(inspection.generated_metadata_hash.is_some());
}

#[tokio::test]
async fn generated_live_run_executes_v2_runtime_and_persists_typed_results() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let client = Arc::new(GeneratedV2RunClient {
        calls: AtomicUsize::new(0),
    });

    let output = run_live_action(
        temp.path(),
        CommandAction::Run {
            decomposed: false,
            task: "Inspect this repository with a generated V2 workflow".to_string(),
        },
        client.clone(),
        tui_tx,
        None,
        default_generated_workflow_config(),
        true,
        LiveApprovalMode::CliYes,
    )
    .await
    .expect("generated run should execute through V2 runtime");

    assert!(output.contains("Workflow V2 complete:"));
    assert!(output.contains("v2_results:"));
    assert_eq!(client.calls.load(Ordering::SeqCst), 2);

    let workflow_root = temp.path().join(".archon/workflows");
    let run_dir = std::fs::read_dir(&workflow_root)
        .expect("workflow root exists")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| path.join("workflow.js").exists())
        .expect("generated workflow run directory");
    let workflow_js = std::fs::read_to_string(run_dir.join("workflow.js")).expect("workflow js");
    let generated_metadata_body =
        std::fs::read_to_string(run_dir.join("v2/generated-metadata.json"))
            .expect("generated v2 metadata");
    let generated_metadata: serde_json::Value =
        serde_json::from_str(&generated_metadata_body).expect("generated metadata json");
    assert_eq!(
        generated_metadata["scaffold_hash"],
        workflow_scaffold_hash(&workflow_js)
    );
    assert!(generated_metadata.get("generated_kind").is_none());
    assert!(generated_metadata.get("generated_scaffold").is_none());
    assert!(run_dir.join("v2/checkpoint.json").exists());

    let result_entries = std::fs::read_dir(run_dir.join("v2/results"))
        .expect("v2 result directory")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("v2 result entries");
    assert_eq!(result_entries.len(), 1);
    let record_body = std::fs::read_to_string(result_entries[0].path()).expect("v2 result body");
    let record: serde_json::Value = serde_json::from_str(&record_body).expect("v2 result json");
    assert_eq!(record["call"]["id"], "inspect");
    assert_eq!(record["status"], "accepted");

    assert!(output.contains("generated_learning:"));
    let learning_body =
        std::fs::read_to_string(run_dir.join("learning/generated-workflow-events.jsonl"))
            .expect("generated learning event log");
    let learning_event: serde_json::Value =
        serde_json::from_str(learning_body.lines().next().expect("learning event line"))
            .expect("learning event json");
    assert_eq!(
        learning_event["schema_version"],
        "workflow-generated-learning-event-v1"
    );
    assert_eq!(learning_event["terminal_status"], "accepted");
    assert_eq!(
        learning_event["scaffold_hash"],
        generated_metadata["scaffold_hash"]
    );
    assert_eq!(learning_event["call_status_counts"]["accepted"], 1);
    assert_eq!(learning_event["canary_result"], "accepted");
}

#[tokio::test]
async fn saved_v2_template_runs_through_v2_runtime() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let harness = r#"
export default async function workflow(w) {
  await w.agent("inspect", { role: "researcher", task: "Inspect via saved V2 command." });
}
"#;
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "saved-v2".to_string(),
        task: "Inspect via saved V2 command".to_string(),
        target_repository_root: None,
        max_parallelism: 4,
        max_agents: 16,
        stages: vec![archon_workflow::StageSpec {
            id: "inspect".to_string(),
            kind: StageKind::Agent,
            task: Some("Approval metadata for saved V2 inspect call".to_string()),
            agent: None,
            foreach: None,
            reducer: None,
            tool: None,
            depends_on: Vec::new(),
            provider_tier: Some(ProviderTier::Researcher),
            retry: archon_workflow::RetryPolicy::default(),
            input: serde_json::json!({
                "runtime": "script_first_v2",
                "metadata_only": true,
                "host_call": "agent"
            }),
            model: None,
            provider: None,
            expected_target_files: Vec::new(),
            verify_command: None,
            max_parallelism: None,
            item_kind: None,
            filter: None,
            extra: Default::default(),
        }],
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    };
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec).expect("seed run");
    WorkflowBundle::create_for_run(
        &store,
        &run,
        harness,
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .expect("seed v2 bundle");
    WorkflowCommandRegistry::project(temp.path())
        .save_run("saved-v2", &store, &run)
        .expect("save v2 command");
    let client = Arc::new(SavedV2TemplateRunClient {
        calls: AtomicUsize::new(0),
    });

    let output = run_live_action(
        temp.path(),
        CommandAction::RunTemplate {
            name: "saved-v2".to_string(),
            args: None,
        },
        client.clone(),
        tui_tx,
        None,
        default_generated_workflow_config(),
        true,
        LiveApprovalMode::CliYes,
    )
    .await
    .expect("saved v2 command should execute");

    assert!(output.contains("Workflow V2 complete:"), "{output}");
    assert!(output.contains("v2_results:"), "{output}");
    assert_eq!(client.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn generated_live_run_executes_read_only_fanout_in_parallel() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let client = Arc::new(GeneratedV2FanoutRunClient {
        calls: AtomicUsize::new(0),
        active_branches: AtomicUsize::new(0),
        peak_branches: AtomicUsize::new(0),
        reduce_source_seen: AtomicUsize::new(0),
    });

    let output = run_live_action(
        temp.path(),
        CommandAction::Run {
            decomposed: false,
            task: "Inspect and fan out over typed items".to_string(),
        },
        client.clone(),
        tui_tx,
        None,
        default_generated_workflow_config(),
        true,
        LiveApprovalMode::CliYes,
    )
    .await
    .expect("generated fanout run should execute through V2 runtime");

    assert!(output.contains("Workflow V2 complete:"));
    assert_eq!(client.calls.load(Ordering::SeqCst), 6);
    assert!(
        client.peak_branches.load(Ordering::SeqCst) > 1,
        "fanout branches did not overlap"
    );
    assert_eq!(client.reduce_source_seen.load(Ordering::SeqCst), 1);

    let workflow_root = temp.path().join(".archon/workflows");
    let run_dir = std::fs::read_dir(&workflow_root)
        .expect("workflow root exists")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| path.join("workflow.js").exists())
        .expect("generated workflow run directory");
    let harness_source =
        std::fs::read_to_string(run_dir.join("workflow.js")).expect("generated workflow harness");
    assert!(harness_source.contains("w.fanout"));
    let compiled = std::fs::read_to_string(run_dir.join("workflow.compiled.yaml"))
        .expect("approval metadata workflow spec");
    let spec: WorkflowSpec = serde_yaml_ng::from_str(&compiled).expect("compiled spec parses");
    let final_stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "final")
        .expect("final metadata stage");
    assert!(
        final_stage.depends_on.is_empty(),
        "generated V2 execution must not be controlled by YAML dependencies"
    );
    assert_eq!(final_stage.input["metadata_only"], true);
    assert_eq!(final_stage.input["runtime"], "script_first_v2");

    let review_record = std::fs::read_dir(run_dir.join("v2/results"))
        .expect("v2 result directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| std::fs::read_to_string(entry.path()).expect("v2 result body"))
        .map(|body| serde_json::from_str::<serde_json::Value>(&body).expect("v2 result json"))
        .find(|record| record["call"]["id"] == "review")
        .expect("review fanout aggregate record");

    assert_eq!(review_record["status"], "accepted");
    assert_eq!(
        review_record["result"]["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(
        review_record["result"]["data"]["peak_parallelism"]
            .as_u64()
            .is_some_and(|peak| peak > 1)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn generated_v2_pause_during_read_only_fanout_stops_pending_branch_launch() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(64);
    let temp = tempfile::tempdir().expect("tempdir");
    let client = Arc::new(GeneratedV2SlowFanoutRunClient {
        calls: AtomicUsize::new(0),
        launched_branches: AtomicUsize::new(0),
    });
    let cwd = temp.path().to_path_buf();
    let store = WorkflowStore::project(&cwd);
    let run_client = client.clone();
    let handle = tokio::spawn(async move {
        run_live_action(
            &cwd,
            CommandAction::Run {
                decomposed: false,
                task: "Inspect and slow-review typed items with a generated V2 workflow"
                    .to_string(),
            },
            run_client,
            tui_tx,
            None,
            default_generated_workflow_config(),
            true,
            LiveApprovalMode::CliYes,
        )
        .await
    });

    let run_id = wait_for_generated_run_id(temp.path()).await;
    while client.launched_branches.load(Ordering::SeqCst) < 2 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    LifecycleController::new(store.clone())
        .apply(&run_id, LifecycleAction::Pause)
        .expect("pause workflow");

    let output = handle
        .await
        .expect("join workflow")
        .expect("workflow output");

    assert!(output.contains("Workflow paused:"), "{output}");
    assert!(
        client.launched_branches.load(Ordering::SeqCst) < 20,
        "pause should stop pending V2 fanout branches before every branch launches"
    );
    let paused = store.load_state(&run_id).expect("paused run");
    assert_eq!(paused.status, archon_workflow::RunStatus::Paused);
}

#[tokio::test]
async fn generated_worktree_write_fanout_applies_patch_to_canonical_repo() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(32);
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn original() -> usize { 0 }\n",
    )
    .expect("seed source");
    init_git_repo(&repo);
    let client = Arc::new(GeneratedV2WorktreeRunClient {
        planner_calls: AtomicUsize::new(0),
        agent_calls: AtomicUsize::new(0),
        implementation_cwd: Mutex::new(None),
    });

    let output = run_live_action(
        temp.path(),
        CommandAction::Run {
            decomposed: false,
            task: format!(
                "Implement one worktree fanout change against the repository {}",
                repo.display()
            ),
        },
        client.clone(),
        tui_tx,
        None,
        default_generated_workflow_config(),
        true,
        LiveApprovalMode::CliYes,
    )
    .await
    .expect("worktree V2 run");

    assert!(output.contains("Workflow V2 complete:"), "{output}");
    assert_eq!(client.planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.agent_calls.load(Ordering::SeqCst), 2);
    let implementation_cwd = client
        .implementation_cwd
        .lock()
        .expect("cwd lock")
        .clone()
        .expect("implementation cwd");
    assert_ne!(implementation_cwd, repo);
    assert!(
        implementation_cwd
            .to_string_lossy()
            .replace('\\', "/")
            .contains("worktrees/implementation"),
        "{}",
        implementation_cwd.display()
    );

    let run_dir = std::fs::read_dir(temp.path().join(".archon/workflows"))
        .expect("workflow root")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| path.join("workflow.js").exists())
        .expect("run directory");
    let implementation_record = std::fs::read_dir(run_dir.join("v2/results"))
        .expect("v2 results")
        .filter_map(|entry| entry.ok())
        .map(|entry| std::fs::read_to_string(entry.path()).expect("result body"))
        .map(|body| serde_json::from_str::<serde_json::Value>(&body).expect("result json"))
        .find(|record| record["call"]["id"] == "implementation")
        .expect("implementation aggregate record");
    let canonical = std::fs::read_to_string(repo.join("src/lib.rs")).expect("canonical source");
    assert!(
        canonical.contains("generated_worktree_value"),
        "canonical file was not updated\noutput:\n{output}\nimplementation_cwd={}\nimplementation_record={}\ncanonical:\n{canonical}",
        implementation_cwd.display(),
        serde_json::to_string_pretty(&implementation_record).expect("pretty record")
    );
    assert_eq!(
        implementation_record["result"]["data"]["serial_fallback_reason"],
        serde_json::Value::Null
    );
    assert!(
        implementation_record["result"]["data"]["worktree_apply_manifests"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
}

#[tokio::test]
async fn closed_tui_prevents_stage_success_publication() {
    let client = Arc::new(CompletionBlockedAgentClient {
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let (tui_tx, mut tui_rx) =
        archon_tui::event_channel::bounded_tui_event_channel_with_capacity(4);
    let stage_runner = PipelineWorkflowRunner {
        llm: client.clone(),
        tui_tx,
        agent_names: Vec::new(),
        workspace_boundary_supported: false,
    };
    let handle = tokio::spawn(async move { stage_runner.run_stage(request(json!({}))).await });

    client.started.notified().await;
    let _running = tui_rx.recv().await.expect("running activity");
    drop(tui_rx);
    client.release.notify_one();

    handle
        .await
        .expect("stage join")
        .expect_err("closed TUI must prevent stage success publication");
}

#[tokio::test]
async fn closed_tui_prevents_item_output_repair_launch() {
    let client = Arc::new(BlockedInvalidItemsAgentClient {
        calls: AtomicUsize::new(0),
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let (tui_tx, mut tui_rx) =
        archon_tui::event_channel::bounded_tui_event_channel_with_capacity(4);
    let stage_runner = PipelineWorkflowRunner {
        llm: client.clone(),
        tui_tx,
        agent_names: Vec::new(),
        workspace_boundary_supported: false,
    };
    let req = StageRunRequest {
        stage_id: "discover".into(),
        stage_kind: StageKind::Agent,
        provider_tier: ProviderTier::Planner,
        task: "Produce implementation items.".into(),
        ..request(json!({
            "stage_extra": { "outputs": ["items"] }
        }))
    };

    let handle = tokio::spawn(async move { stage_runner.run_stage(req).await });
    let _running = tui_rx.recv().await.expect("initial running activity");
    client.started.notified().await;
    drop(tui_rx);
    client.release.notify_one();

    handle
        .await
        .expect("stage join")
        .expect_err("closed TUI must prevent item-output repair launch");
    assert_eq!(client.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn closed_tui_prevents_stage_agent_launch() {
    let client = Arc::new(FlakyAgentClient {
        calls: AtomicUsize::new(0),
        first_error: "unused",
    });
    let (tui_tx, tui_rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    drop(tui_rx);
    let stage_runner = PipelineWorkflowRunner {
        llm: client.clone(),
        tui_tx,
        agent_names: Vec::new(),
        workspace_boundary_supported: false,
    };

    stage_runner
        .run_stage(request(json!({})))
        .await
        .expect_err("closed TUI must prevent stage agent launch");

    assert_eq!(client.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn workflow_live_retries_transient_agent_decode_errors() {
    let client = Arc::new(FlakyAgentClient {
        calls: AtomicUsize::new(0),
        first_error: "HTTP error: http_error: HTTP error: error decoding response body",
    });
    let (stage_runner, _tui_rx) = runner(client.clone());

    let output = stage_runner
        .run_stage(request(json!({
            "target_repository_root": "/tmp/target-repo",
        })))
        .await
        .expect("transient provider decode failures should retry and recover");

    assert_eq!(output.body, "status: completed");
    assert_eq!(client.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn workflow_live_does_not_retry_permission_errors() {
    let client = Arc::new(FlakyAgentClient {
        calls: AtomicUsize::new(0),
        first_error: "bypassPermissions requires --allow-dangerously-skip-permissions flag",
    });
    let (stage_runner, _tui_rx) = runner(client.clone());

    let err = stage_runner
        .run_stage(request(json!({})))
        .await
        .expect_err("permission/config failures are not transport transients");

    assert!(
        err.to_string()
            .contains("bypassPermissions requires --allow-dangerously-skip-permissions")
    );
    assert_eq!(client.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn workflow_live_repairs_invalid_item_producer_output_once() {
    let client = Arc::new(InvalidItemsThenRepairAgentClient {
        calls: AtomicUsize::new(0),
        requests: Mutex::new(Vec::new()),
    });
    let (stage_runner, _tui_rx) = runner(client.clone());
    let req = StageRunRequest {
        stage_id: "discover".into(),
        stage_kind: StageKind::Agent,
        provider_tier: ProviderTier::Planner,
        task: "Produce implementation items.".into(),
        ..request(json!({
            "stage_extra": {
                "outputs": ["items"]
            }
        }))
    };

    let output = stage_runner
        .run_stage(req)
        .await
        .expect("invalid item output should get one schema repair attempt");

    assert!(output.body.contains(r#""items""#));
    assert_eq!(client.calls.load(Ordering::SeqCst), 2);
    let requests = client.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].session_id, "wf-test-stage-discover-attempt-1");
    assert_eq!(requests[0].session_id, requests[1].session_id);
    let repair_prompt = requests[1].messages[0]["content"]
        .as_str()
        .expect("repair prompt content");
    assert!(repair_prompt.contains("declares `outputs: [items]`"));
    assert!(repair_prompt.contains("Return ONLY one JSON object"));
    assert!(repair_prompt.contains("Do not return restored-context summaries"));
}

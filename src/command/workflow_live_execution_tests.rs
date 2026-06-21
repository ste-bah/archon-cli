use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::{
    CommandAction, LifecycleAction, LifecycleController, ProviderTier, StageKind, StageRunRequest,
    WorkflowBundle, WorkflowBundleOrigin, WorkflowCommandRegistry, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore, WorkflowV2HarnessValidator,
};
use serde_json::json;

use super::workflow_live_retry::transient_live_agent_error;
use super::workflow_live_test_support::{
    AlwaysInvalidItemsAgentClient, FlakyAgentClient, FlakyPlanner, GeneratedV2FanoutRunClient,
    GeneratedV2RunClient, GeneratedV2SlowFanoutRunClient, GeneratedV2WorktreeRunClient,
    GuttedImplementationPlanner, InvalidItemsThenRepairAgentClient, InvalidPlanner,
    SavedV2TemplateRunClient, request, runner,
};
use super::{LiveApprovalMode, plan_live, run_live_action};

#[tokio::test]
async fn live_planner_validation_failure_does_not_fallback_to_smoke_plan() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let err = plan_live(
        &store,
        "implement the whole PRD",
        Arc::new(InvalidPlanner),
        tui_tx,
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
    assert!(value["attempts"][0]["content_preview"].as_str().is_some());
}

#[tokio::test]
async fn live_planner_retries_transient_stream_server_errors() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let planner = Arc::new(FlakyPlanner {
        calls: AtomicUsize::new(0),
        first_error: "LLM stream error (server_error): temporary upstream failure",
    });

    let plan = plan_live(&store, "inspect the repository", planner.clone(), tui_tx)
        .await
        .expect("transient planner stream failure should retry and recover");

    assert_eq!(plan.calls.len(), 1);
    assert_eq!(plan.calls[0].id, "discover");
    assert_eq!(planner.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn implementation_prd_plan_uses_provider_generated_harness_not_deterministic_scaffold() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let planner = Arc::new(GuttedImplementationPlanner {
        calls: AtomicUsize::new(0),
    });

    let plan = plan_live(
        &store,
        "Implement the decomposed PRD at /tmp/tasks/PRD-EXAMPLE-001 against the repository /tmp/repo",
        planner.clone(),
        tui_tx,
    )
    .await
    .expect("decomposed PRD implementation should use provider-generated harness");

    assert_ne!(planner.calls.load(Ordering::SeqCst), 0);
    assert!(plan.calls.len() < 24);
    let implementation = plan
        .calls
        .iter()
        .find(|call| call.id == "implementationResults")
        .expect("implementation fanout call");
    assert_eq!(
        implementation.write_mode,
        Some(archon_workflow::WorkflowV2WriteMode::Coordinated)
    );
    assert_eq!(
        implementation.options.source.as_deref(),
        Some("inventory.items")
    );
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
            task: "Inspect this repository with a generated V2 workflow".to_string(),
        },
        client.clone(),
        tui_tx,
        None,
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
    let v2_plan = WorkflowV2HarnessValidator
        .validate(harness)
        .expect("v2 harness validates");
    let spec = super::workflow_live_compat::compatibility_spec_from_v2_calls(
        "Inspect via saved V2 command",
        &v2_plan.calls,
    );
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
        },
        client.clone(),
        tui_tx,
        None,
        true,
        LiveApprovalMode::CliYes,
    )
    .await
    .expect("saved v2 command should execute");

    assert!(output.contains("Workflow V2 complete:"), "{output}");
    assert!(output.contains("v2_results:"), "{output}");
    assert_eq!(client.calls.load(Ordering::SeqCst), 1);
}

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
            task: "Inspect and fan out over typed items".to_string(),
        },
        client.clone(),
        tui_tx,
        None,
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
    let compiled = std::fs::read_to_string(run_dir.join("workflow.compiled.yaml"))
        .expect("compiled workflow spec");
    let spec: WorkflowSpec = serde_yaml_ng::from_str(&compiled).expect("compiled spec parses");
    let final_stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "final")
        .expect("final reduce stage");
    assert_eq!(final_stage.depends_on, vec!["inventory", "review"]);

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
                task: "Inspect and slow-review typed items with a generated V2 workflow"
                    .to_string(),
            },
            run_client,
            tui_tx,
            None,
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
            task: format!(
                "Implement one worktree fanout change against the repository {}",
                repo.display()
            ),
        },
        client.clone(),
        tui_tx,
        None,
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
async fn workflow_live_retries_transient_agent_decode_errors() {
    let client = Arc::new(FlakyAgentClient {
        calls: AtomicUsize::new(0),
        first_error: "HTTP error: http_error: HTTP error: error decoding response body",
    });
    let stage_runner = runner(client.clone());

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
    let stage_runner = runner(client.clone());

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
    let stage_runner = runner(client.clone());
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

#[tokio::test]
async fn workflow_live_fails_item_producer_when_repair_is_still_invalid() {
    let client = Arc::new(AlwaysInvalidItemsAgentClient {
        calls: AtomicUsize::new(0),
    });
    let stage_runner = runner(client.clone());
    let req = StageRunRequest {
        stage_id: "t001-inventory".into(),
        stage_kind: StageKind::Agent,
        provider_tier: ProviderTier::Planner,
        task: "Produce implementation items.".into(),
        depends_on: vec!["read-audit".into()],
        ..request(json!({
            "stage_extra": {
                "outputs": ["items"]
            }
        }))
    };

    let err = stage_runner
        .run_stage(req)
        .await
        .expect_err("still-invalid item producer output must fail clearly");

    assert!(err.to_string().contains("after schema repair retry"));
    assert_eq!(client.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn workflow_live_recovers_read_only_discovery_items_when_repair_is_still_invalid() {
    let client = Arc::new(AlwaysInvalidItemsAgentClient {
        calls: AtomicUsize::new(0),
    });
    let stage_runner = runner(client.clone());
    let req = StageRunRequest {
        stage_id: "discover".into(),
        stage_kind: StageKind::Agent,
        provider_tier: ProviderTier::Planner,
        task: "Read /tmp/repo/README.md and /tmp/repo/tasks/TASK-001.md before planning.".into(),
        ..request(json!({
            "target_repository_root": "/tmp/repo",
            "stage_extra": {
                "outputs": ["items"]
            }
        }))
    };

    let output = stage_runner
        .run_stage(req)
        .await
        .expect("dependency-free read-only discovery should recover into concrete source items");

    let value: serde_json::Value = serde_json::from_str(&output.body).expect("fallback json");
    let items = value["items"].as_array().expect("items array");
    assert!(!items.is_empty());
    assert!(
        items
            .iter()
            .any(|item| item["path"] == "/tmp/repo/README.md")
    );
    assert!(items.iter().all(|item| {
        item["dependency_order_notes"]
            .as_str()
            .is_some_and(|note| note.contains("no implementation completion is claimed"))
    }));
    assert_eq!(
        value["runtime_recovery"]["kind"],
        "read_only_discovery_items"
    );
    assert!(
        value["runtime_recovery"]["strictness"]
            .as_str()
            .is_some_and(|strictness| strictness.contains("implementation"))
    );
    assert_eq!(client.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn transient_classifier_matches_provider_decode_but_not_permission_errors() {
    assert!(transient_live_agent_error(
        "LLM stream error (server_error): temporary upstream failure"
    ));
    assert!(transient_live_agent_error(
        "HTTP error: http_error: HTTP error: error decoding response body"
    ));
    assert!(!transient_live_agent_error(
        "bypassPermissions requires --allow-dangerously-skip-permissions flag"
    ));
}

fn init_git_repo(repo: &std::path::Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.name", "archon-test"]);
    run_git(
        repo,
        &["config", "user.email", "archon-test@example.invalid"],
    );
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command starts");
    assert!(
        output.status.success(),
        "git {:?} failed: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn wait_for_generated_run_id(cwd: &std::path::Path) -> String {
    let workflow_root = cwd.join(".archon/workflows");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(entries) = std::fs::read_dir(&workflow_root) {
            if let Some(run_id) = entries.filter_map(|entry| entry.ok()).find_map(|entry| {
                let path = entry.path();
                if path.join("workflow.js").exists() {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_string)
                } else {
                    None
                }
            }) {
                return run_id;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "generated workflow run directory was not created"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

use super::*;

#[tokio::test]
async fn workflow_live_fails_item_producer_when_repair_is_still_invalid() {
    let client = Arc::new(AlwaysInvalidItemsAgentClient {
        calls: AtomicUsize::new(0),
    });
    let (stage_runner, _tui_rx) = runner(client.clone());
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
    let (stage_runner, _tui_rx) = runner(client.clone());
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

pub(super) fn init_git_repo(repo: &std::path::Path) {
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

pub(super) async fn wait_for_generated_run_id(cwd: &std::path::Path) -> String {
    let workflow_root = cwd.join(".archon/workflows");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(entries) = std::fs::read_dir(&workflow_root)
            && let Some(run_id) = entries.filter_map(|entry| entry.ok()).find_map(|entry| {
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
        assert!(
            std::time::Instant::now() < deadline,
            "generated workflow run directory was not created"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

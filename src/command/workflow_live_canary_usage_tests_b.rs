fn print_canary_evidence(
    rows: &[archon_learning::llm_call_usage::LlmCallUsageRecord],
    request_bytes: &[u64],
) {
    let totals = rows.iter().fold((0, 0, 0, 0), |totals, row| {
        (
            totals.0 + known_usage(&row.input_tokens),
            totals.1 + known_usage(&row.cache_creation_input_tokens),
            totals.2 + known_usage(&row.cache_read_input_tokens),
            totals.3 + known_usage(&row.output_tokens),
        )
    });
    let evidence = serde_json::json!({
        "fixture": "canary_wf_afae6bee_regression",
        "measurement": "controlled provider-shaped serialized-request-byte/4 estimates",
        "measurement_overlay": true,
        "external_provider_telemetry": false,
        "source_revision": option_env!("GIT_SHA").unwrap_or("unknown"),
        "call_count": rows.len(),
        "request_bytes": request_bytes,
        "usage_totals": {
            "input_tokens": totals.0,
            "cache_creation_input_tokens": totals.1,
            "cache_read_input_tokens": totals.2,
            "output_tokens": totals.3,
        }
    });
    println!("ISSUE75_CANARY_LEDGER_EVIDENCE={evidence}");
}

fn known_usage(usage: &UsageAvailability) -> u64 {
    match usage {
        UsageAvailability::Known(value) => *value,
        UsageAvailability::Unavailable => panic!("controlled provider usage must be available"),
    }
}

fn canary_git(repo: &std::path::Path, args: &[&str]) {
    let output = CanaryGitCommand::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command starts");
    assert!(
        output.status.success(),
        "git {:?} failed: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn seed_canary_project(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    std::fs::write(repo.join("src/lib.rs"), "pub fn gap_audit() {}\n").expect("seed source");
    canary_git(&repo, &["init"]);
    canary_git(&repo, &["config", "user.name", "archon-canary"]);
    canary_git(&repo, &["config", "user.email", "canary@example.invalid"]);
    canary_git(&repo, &["add", "."]);
    canary_git(&repo, &["commit", "-m", "initial"]);
    let tasks = root.join("tasks/PRD-CANARY-AFAE6BEE-001");
    std::fs::create_dir_all(&tasks).expect("task dir");
    std::fs::write(
        tasks.join("TASK-TDL-001-data-lake-gap-audit.md"),
        format!(
            "# Data Lake Gap Audit\n\ntask_id: TASK-TDL-001\ndepends_on: []\n\n\
             ## Acceptance Criteria\n\n- Gap audit implemented in the target repository.\n\
             - Artifact evidence written to `{CANARY_ARTIFACT_REL}`.\n\n\
             ## Artifact Requirements\n\n- `{CANARY_ARTIFACT_REL}`\n"
        ),
    )
    .expect("task file");
    (repo, tasks)
}

struct CanaryRunHarness {
    script: Arc<CanaryAgentClient>,
    request_bytes: Arc<Mutex<Vec<u64>>>,
    client: Arc<dyn LlmClient>,
}

async fn build_canary_harness(root: &std::path::Path) -> CanaryRunHarness {
    let script = Arc::new(CanaryAgentClient::new(root.to_path_buf()));
    let request_bytes = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn LlmProvider> = Arc::new(CanaryProvider::new(
        Arc::clone(&script),
        Arc::clone(&request_bytes),
    ));
    let provider = crate::runtime::provider_observer::observe_llm_provider_with_profile(
        provider,
        "workflow-canary",
        None,
    )
    .await;
    install_canary_executor(Arc::clone(&provider), root);
    let raw: Arc<dyn LlmClient> =
        Arc::new(ProviderLlmAdapter::new(Arc::clone(&provider)).with_origin("workflow-canary"));
    let fallback: Arc<dyn LlmClient> = Arc::new(ScopedCanaryClient::new(raw));
    let client = Arc::new(SubagentPipelineClient::with_provider(
        fallback,
        ToolContext {
            working_dir: root.to_path_buf(),
            ..ToolContext::default()
        },
        provider,
    ));
    CanaryRunHarness {
        script,
        request_bytes,
        client,
    }
}

fn assert_canary_output(output: &str, script: &CanaryAgentClient) -> usize {
    let prompts = script.prompts.lock().expect("prompt log").clone();
    assert!(
        script.artifact_exists(),
        "artifact contract did not reach implementation prompt. Prompts: {}\nOutput:\n{output}",
        prompts.join("\n---\n"),
    );
    assert!(!output.contains("blocked-verification-failed"), "{output}");
    assert!(
        output.contains("Workflow V2 complete:")
            || (output.contains("Workflow V2 needs review:")
                && output.contains("failed_call: blocked-final-readiness")),
        "{output}"
    );
    prompts.len()
}

async fn run_canary() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(64);
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let learning_db_path = root.join(".archon").join("learning-state.db");
    // SAFETY: this fixture executes in an isolated child process.
    unsafe {
        std::env::set_var("ARCHON_LEARNING_DB_PATH", &learning_db_path);
    }
    let (repo, tasks) = seed_canary_project(root);
    let task = format!(
        "Implement the decomposed PRD at {} against the repository {}",
        tasks.display(),
        repo.display()
    );
    let harness = build_canary_harness(root).await;
    let output = run_live_action(
        root,
        // Pin the engine: `decomposed: false` defers to
        // `script_lifecycle_from_env()`, whose default has since flipped to the
        // v3 authored-script lifecycle, which `CanaryAgentClient` cannot script
        // (no author branch => generic evidence => `authoring_failed`).
        CommandAction::Run {
            task,
            decomposed: true,
        },
        harness.client,
        tui_tx,
        None,
        archon_core::config::GeneratedWorkflowConfig::default(),
        true,
        LiveApprovalMode::CliYes,
    )
    .await
    .expect("decomposed PRD canary run completes with a final report");
    let prompt_count = assert_canary_output(&output, &harness.script);
    let request_bytes = harness
        .request_bytes
        .lock()
        .expect("request byte log lock")
        .clone();
    assert_canary_usage(&learning_db_path, prompt_count, &request_bytes);
}

fn run_isolated_child() {
    let status =
        std::process::Command::new(std::env::current_exe().expect("current test executable"))
            .arg("--exact")
            .arg(CANARY_TEST)
            .arg("--nocapture")
            .env(CANARY_CHILD_ENV, "execute")
            .status()
            .expect("run isolated canary child");
    assert!(status.success(), "isolated canary child failed");
}

#[test]
fn canary_wf_afae6bee_provider_ledger() {
    match std::env::var(CANARY_CHILD_ENV) {
        Ok(value) => {
            assert_eq!(value, "execute", "unexpected child marker");
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("canary runtime")
                .block_on(run_canary());
        }
        Err(std::env::VarError::NotPresent) => run_isolated_child(),
        Err(error) => panic!("invalid child marker: {error}"),
    }
}

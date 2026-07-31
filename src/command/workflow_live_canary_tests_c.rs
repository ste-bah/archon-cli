fn collect_text(value: &serde_json::Value, into: &mut String) {
    match value {
        serde_json::Value::String(text) => {
            into.push_str(text);
            into.push('\n');
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_text(item, into);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_text(item, into);
            }
        }
        _ => {}
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

#[tokio::test]
async fn canary_wf_afae6bee_regression() {
    let (_lifecycle_lock, _lifecycle_env) = DecomposedLifecycleEnvGuard::set().await;
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(64);
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path();

    let repo = project_root.join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    std::fs::write(repo.join("src/lib.rs"), "pub fn gap_audit() {}\n").expect("seed source");
    canary_git(&repo, &["init"]);
    canary_git(&repo, &["config", "user.name", "archon-canary"]);
    canary_git(&repo, &["config", "user.email", "canary@example.invalid"]);
    canary_git(&repo, &["add", "."]);
    canary_git(&repo, &["commit", "-m", "initial"]);

    let tasks = project_root.join("tasks/PRD-CANARY-AFAE6BEE-001");
    std::fs::create_dir_all(&tasks).expect("task dir");
    std::fs::write(
        tasks.join("TASK-TDL-001-data-lake-gap-audit.md"),
        format!(
            "# Data Lake Gap Audit\n\n\
             task_id: TASK-TDL-001\n\
             depends_on: []\n\n\
             ## Acceptance Criteria\n\n\
             - Gap audit implemented in the target repository.\n\
             - Artifact evidence written to `{CANARY_ARTIFACT_REL}`.\n\n\
             ## Artifact Requirements\n\n\
             - `{CANARY_ARTIFACT_REL}`\n"
        ),
    )
    .expect("task file");

    let task = format!(
        "Implement the decomposed PRD at {} against the repository {}",
        tasks.display(),
        repo.display()
    );
    let client = Arc::new(CanaryAgentClient::new(project_root.to_path_buf()));

    let output = run_live_action(
        project_root,
        CommandAction::Run {
            task,
            decomposed: false,
        },
        client.clone(),
        tui_tx,
        None,
        archon_core::config::GeneratedWorkflowConfig::default(),
        true,
        LiveApprovalMode::CliYes,
    )
    .await
    .expect("decomposed PRD canary run completes with a final report");

    let prompts = client.prompts.lock().expect("prompt log").clone();

    assert!(
        client.artifact_exists(),
        "wf-afae6bee regression: the task pack declares artifact evidence at \
         `{CANARY_ARTIFACT_REL}`, but no implementing agent was ever instructed \
         to write it (declared artifact contract never reached agent prompts). \
         Prompts seen ({}):\n{}",
        prompts.len(),
        prompts.join("\n---\n"),
    );
    assert!(
        !output.contains("blocked-verification-failed"),
        "wf-afae6bee regression: run latched into a run-level verification \
         block instead of completing or failing a single call with a \
         structured error. Output:\n{output}",
    );
    let terminal_report = output.contains("Workflow V2 complete:")
        || (output.contains("Workflow V2 needs review:")
            && output.contains("failed_call: blocked-final-readiness"));
    assert!(
        terminal_report,
        "canary run must end with final acceptance or an explicit final-readiness block. Output:\n{output}",
    );
}

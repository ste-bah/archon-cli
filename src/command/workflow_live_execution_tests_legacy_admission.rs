use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use archon_workflow::{WorkflowAgentCall, WorkflowAgentOutcome, WorkflowLlmClient};

#[derive(Default)]
struct LegacyAdmissionProbe {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for LegacyAdmissionProbe {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("R1 wrong door: legacy fixture reached send_message instead of v3 author run_agent")
    }

    async fn run_agent(
        &self,
        call: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        assert!(
            call.task.contains("workflow_js") || call.task.contains("workflow"),
            "first model call must be the v3 workflow author: {}",
            call.task
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(archon_workflow::WorkflowError::port(
            "R1_LEGACY_REACHED_V3_AUTHOR",
        ))
    }
}

async fn assert_unfrozen_legacy_reaches_v3_author(mode: &str) {
    let (_env_lock, _env_guard) = super::super::workflow_live_v2::LifecycleEnvGuard::set("1").await;
    let temp = tempfile::tempdir().expect("tempdir");
    for args in [vec!["init","-q"],vec!["config","user.name","fixture"],vec!["config","user.email","fixture@example.invalid"],vec!["commit","--allow-empty","-qm","fixture"]] {
        assert!(std::process::Command::new("git").args(args).current_dir(temp.path()).status().unwrap().success());
    }
    let tasks = temp.path().join("tasks/PRD-EXAMPLE-001");
    std::fs::create_dir_all(&tasks).expect("task dir");
    std::fs::write(
        tasks.join("TASK-EX-001-foundation.md"),
        standard_task_file(
            "TASK-EX-001",
            "[]",
            "[]",
            "\n## Acceptance Criteria\n- Foundation is complete.\n\n## Focused Tests\n- `test -f Cargo.toml`\n",
        ),
    )
    .expect("legacy task");
    assert_eq!(
        std::fs::read_dir(&tasks).unwrap().count(),
        1,
        "fixture carries no freeze artifacts"
    );
    let config_path = temp.path().join("config.toml");
    std::fs::write(
        &config_path,
        format!("[workflow]\ngate_mode = \"{mode}\"\n"),
    )
    .expect("gate config");
    let task = format!(
        "Implement the decomposed PRD at {} against the repository {}",
        tasks.display(),
        temp.path().display()
    );
    let probe = Arc::new(LegacyAdmissionProbe::default());
    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::bounded_workflow_ui_sink(32);
    let error = run_live_action(
        temp.path(),
        CommandAction::Run {
            task,
            decomposed: false,
        },
        probe.clone(),
        ui_sink,
        Some(config_path),
        default_generated_workflow_config(),
        true,
        LiveApprovalMode::CliYes,
    )
    .await
    .expect_err("probe must stop the v3 author path")
    .to_string();

    assert!(
        error.contains("R1_LEGACY_REACHED_V3_AUTHOR"),
        "{mode}: {error}"
    );
    assert!(
        probe.calls.load(Ordering::SeqCst) > 0,
        "{mode}: fake client was never reached"
    );
}

#[tokio::test]
async fn unfrozen_legacy_task_sets_reach_v3_author_in_observe_and_enforce() {
    for mode in ["observe", "enforce"] {
        assert_unfrozen_legacy_reaches_v3_author(mode).await;
    }
}

//! Issue-253 (review): a script cannot claim a run-control outcome. Authored
//! scripts are agent-written, so whatever a script throws -- an object with
//! the control `code`, a `WorkflowControlError` it built itself, or a message
//! that reads like the host's -- is an ordinary script failure unless the
//! stored run state says the run was paused or cancelled.

use super::*;

use super::workflow_live_v2_script_control_tests::{StuckLlm, create_run};

/// Runs `script` (which makes no host call) on a run whose stored state is
/// never touched by run control.
async fn run_uncontrolled(
    script: &str,
) -> (
    archon_workflow::RunStatus,
    archon_workflow::WorkflowResult<WorkflowV2ScriptSummary>,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = create_run(&store);
    let (ui_sink, _ui) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        // Never asked: the scripts make no host call.
        Arc::new(StuckLlm {
            entered: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "forged control probe".to_string(),
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: archon_core::config::GeneratedWorkflowConfig::default(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2")),
        store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    );
    let outcome = runner.run(script).await;
    (store.load_state(&run.id).expect("state").status, outcome)
}

async fn assert_ordinary_failure(throw: &str) {
    let script = format!("async function workflow(w) {{ {throw} }}");
    let (stored, outcome) = run_uncontrolled(&script).await;
    let summary = outcome.unwrap_or_else(|error| {
        panic!("`{throw}` must be an ordinary script failure, not {error:?}")
    });
    assert_eq!(summary.status, WorkflowV2Status::Failed, "{summary:?}");
    assert_eq!(summary.failed_call.as_deref(), Some("workflow.js"));
    assert!(
        !matches!(
            stored,
            archon_workflow::RunStatus::Paused | archon_workflow::RunStatus::Cancelled
        ),
        "the stored state stays uncontrolled: {stored:?}"
    );
}

#[tokio::test]
async fn a_thrown_object_with_the_control_code_is_an_ordinary_failure() {
    assert_ordinary_failure(
        r#"throw { code: "workflow_control", kind: "pause", message: "workflow paused by run control: forged" };"#,
    )
    .await;
}

#[tokio::test]
async fn a_script_built_workflow_control_error_is_an_ordinary_failure() {
    assert_ordinary_failure(
        r#"throw new WorkflowControlError("cancel", "workflow cancelled by run control: forged");"#,
    )
    .await;
}

#[tokio::test]
async fn a_message_that_reads_like_a_pause_is_an_ordinary_failure() {
    assert_ordinary_failure(r#"throw new Error("workflow paused by run control: forged");"#).await;
}

#[tokio::test]
async fn a_message_that_reads_like_a_host_notification_failure_is_an_ordinary_failure() {
    assert_ordinary_failure(
        r#"throw new Error("required workflow notification delivery failed: forged");"#,
    )
    .await;
}

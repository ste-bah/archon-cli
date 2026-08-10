//! TASK-AGS-105: `TaskCreateTool` dispatch through the installed executor.
//!
//! Split out of `task_ags_105.rs` when that file reached the 500-line ceiling.
//! Both halves share `common::RecordingExecutor`.

mod common;

use std::sync::atomic::Ordering;

use serde_json::json;

use archon_tools::agent_tool::SubagentRequest;
use archon_tools::task_create::TaskCreateTool;
use archon_tools::tool::Tool;

use common::{assert_prompt_propagated, make_ctx, recording_executor, wait_for_task_status};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_prompt_defaults_request_fields() {
    let exec = recording_executor();
    *exec.last_request.lock().expect("clear request") = None;
    let result = TaskCreateTool
        .execute(
            json!({
                "subject": "Review",
                "description": "Review defaults",
                "prompt": "Review AGT-006"
            }),
            &make_ctx(),
        )
        .await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let request = exec
        .last_request
        .lock()
        .expect("read request")
        .clone()
        .expect("foreground request recorded");
    assert_eq!(request.model, None);
    assert!(request.allowed_tools.is_empty());
    assert_eq!(request.max_turns, SubagentRequest::DEFAULT_MAX_TURNS);
    assert_eq!(request.timeout_secs, SubagentRequest::DEFAULT_TIMEOUT_SECS);
    assert_eq!(request.subagent_type, None);
    assert!(!request.run_in_background);
    assert_eq!(request.cwd, None);
    assert_eq!(request.isolation, None);
    assert_eq!(request.provider_env, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_foreground_propagates_request_fields() {
    let exec = recording_executor();
    *exec.last_request.lock().expect("clear request") = None;
    exec.last_nested.store(false, Ordering::SeqCst);
    // The executor is a process-wide singleton and the background test below
    // raises both of these, restoring them on its last two lines — which it
    // never reaches when it fails. A leftover auto-background window silently
    // turns this foreground run into an auto-backgrounded one, so this test
    // owns the state it depends on rather than inheriting it.
    exec.auto_bg_ms.store(0, Ordering::SeqCst);
    exec.run_delay_ms.store(0, Ordering::SeqCst);
    let tool = TaskCreateTool;
    let input = json!({
        "subject": "Review",
        "description": "Review agent wiring",
        "prompt": "Review AGT-006",
        "model": "sonnet",
        "allowed_tools": ["Read", "Grep"],
        "subagent_type": "code-reviewer",
        "run_in_background": false,
        "cwd": "/tmp"
    });

    let result = tool.execute(input, &make_ctx()).await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).expect("response json");
    assert!(response["task_id"].is_string());
    assert_eq!(response["result"], "recorded");
    assert!(exec.ran.load(Ordering::SeqCst));
    assert!(exec.last_nested.load(Ordering::SeqCst));
    let request = exec
        .last_request
        .lock()
        .expect("read request")
        .clone()
        .expect("foreground request recorded");
    assert_prompt_propagated(&request.prompt);
    assert_eq!(request.model.as_deref(), Some("sonnet"));
    assert_eq!(request.allowed_tools, ["Read", "Grep"]);
    assert_eq!(request.subagent_type.as_deref(), Some("code-reviewer"));
    assert!(!request.run_in_background);
    assert_eq!(request.cwd.as_deref(), Some("/tmp"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_background_classifies_propagated_request_fields() {
    let exec = recording_executor();
    *exec
        .last_classified_request
        .lock()
        .expect("clear classified request") = None;
    *exec.last_request.lock().expect("clear request") = None;
    exec.auto_bg_ms.store(1, Ordering::SeqCst);
    exec.run_delay_ms.store(50, Ordering::SeqCst);
    let tool = TaskCreateTool;
    let input = json!({
        "subject": "Review",
        "description": "Review in background",
        "prompt": "Review AGT-006",
        "model": "sonnet",
        "allowed_tools": ["Read", "Grep"],
        "subagent_type": "code-reviewer",
        "run_in_background": true,
        "cwd": "/tmp"
    });

    let result = tool.execute(input, &make_ctx()).await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).expect("response json");
    assert!(response["task_id"].is_string());
    let task_id = response["task_id"].as_str().expect("task id").to_string();
    assert!(response["agent_id"].is_string());
    assert_eq!(response["status"], "spawned");
    let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run_started = exec.run_notify.notified();
            if let Some(request) = exec.last_request.lock().expect("read request").clone()
                && request.prompt.starts_with("Review AGT-006")
            {
                break request;
            }
            run_started.await;
        }
    })
    .await
    .expect("TaskCreate background request must reach the installed executor");
    assert_prompt_propagated(&request.prompt);
    assert_eq!(request.model.as_deref(), Some("sonnet"));
    assert_eq!(request.allowed_tools, ["Read", "Grep"]);
    assert_eq!(request.subagent_type.as_deref(), Some("code-reviewer"));
    assert!(request.run_in_background);
    assert_eq!(request.cwd.as_deref(), Some("/tmp"));
    wait_for_task_status(&task_id, archon_tools::task_manager::TaskStatus::Completed).await;
    exec.auto_bg_ms.store(0, Ordering::SeqCst);
    exec.run_delay_ms.store(0, Ordering::SeqCst);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_auto_background_completes_tracked_task() {
    let exec = recording_executor();
    exec.auto_bg_ms.store(1, Ordering::SeqCst);
    exec.run_delay_ms.store(50, Ordering::SeqCst);
    let result = TaskCreateTool
        .execute(
            json!({
                "subject": "Review",
                "description": "Review after auto background",
                "prompt": "Review auto background"
            }),
            &make_ctx(),
        )
        .await;
    exec.auto_bg_ms.store(0, Ordering::SeqCst);
    exec.run_delay_ms.store(0, Ordering::SeqCst);

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).expect("response json");
    assert_eq!(response["status"], "auto_backgrounded");
    let task_id = response["task_id"].as_str().expect("task id");
    wait_for_task_status(task_id, archon_tools::task_manager::TaskStatus::Completed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_auto_background_fails_tracked_task() {
    let exec = recording_executor();
    exec.auto_bg_ms.store(1, Ordering::SeqCst);
    exec.run_delay_ms.store(50, Ordering::SeqCst);
    exec.fail.store(true, Ordering::SeqCst);
    let result = TaskCreateTool
        .execute(
            json!({
                "subject": "Review",
                "description": "Fail after auto background",
                "prompt": "Review auto background failure"
            }),
            &make_ctx(),
        )
        .await;
    exec.auto_bg_ms.store(0, Ordering::SeqCst);
    exec.run_delay_ms.store(0, Ordering::SeqCst);
    exec.fail.store(false, Ordering::SeqCst);

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).expect("response json");
    assert_eq!(response["status"], "auto_backgrounded");
    let task_id = response["task_id"].as_str().expect("task id");
    wait_for_task_status(task_id, archon_tools::task_manager::TaskStatus::Failed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_auto_background_sender_drop_fails_tracked_task() {
    let exec = recording_executor();
    exec.auto_bg_ms.store(1, Ordering::SeqCst);
    exec.run_delay_ms.store(50, Ordering::SeqCst);
    exec.panic.store(true, Ordering::SeqCst);
    let result = TaskCreateTool
        .execute(
            json!({
                "subject": "Review",
                "description": "Panic after auto background",
                "prompt": "Review auto background panic"
            }),
            &make_ctx(),
        )
        .await;
    exec.auto_bg_ms.store(0, Ordering::SeqCst);
    exec.run_delay_ms.store(0, Ordering::SeqCst);
    exec.panic.store(false, Ordering::SeqCst);

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).expect("response json");
    assert_eq!(response["status"], "auto_backgrounded");
    let task_id = response["task_id"].as_str().expect("task id");
    wait_for_task_status(task_id, archon_tools::task_manager::TaskStatus::Failed).await;
}

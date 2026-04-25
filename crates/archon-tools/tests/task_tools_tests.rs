use std::sync::Arc;
use std::thread;

use archon_tools::task_create::TaskCreateTool;
use archon_tools::task_get::TaskGetTool;
use archon_tools::task_list::TaskListTool;
use archon_tools::task_output::TaskOutputTool;
use archon_tools::task_stop::TaskStopTool;
use archon_tools::task_update::TaskUpdateTool;
use archon_tools::tool::{AgentMode, PermissionLevel, Tool, ToolContext};

use archon_tools::task_manager::{TASK_MANAGER, TaskManager, TaskStatus};

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test-session".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// TaskManager unit tests
// ---------------------------------------------------------------------------

#[test]
fn task_create_returns_id() {
    let mgr = TaskManager::new();
    let id = mgr.create_task("test task");
    assert_eq!(id.len(), 8, "task ID should be 8 chars, got: {id}");
    // Should be valid hex
    assert!(
        id.chars().all(|c| c.is_ascii_hexdigit()),
        "task ID should be hex chars: {id}"
    );
}

#[test]
fn task_get_returns_info() {
    let mgr = TaskManager::new();
    let id = mgr.create_task("my task description");
    let info = mgr.get_task(&id).expect("task should exist");
    assert_eq!(info.description, "my task description");
    assert_eq!(info.status, TaskStatus::Pending);
    assert_eq!(info.id, id);
    assert!(info.completed_at.is_none());
    assert!(info.output.is_empty());
    assert_eq!(info.cost, 0.0);
}

#[test]
fn task_update_description() {
    let mgr = TaskManager::new();
    let id = mgr.create_task("old description");
    mgr.update_task(&id, Some("new description"))
        .expect("update should work");
    let info = mgr.get_task(&id).expect("task should exist");
    assert_eq!(info.description, "new description");
}

#[test]
fn task_list_all() {
    let mgr = TaskManager::new();
    mgr.create_task("task 1");
    mgr.create_task("task 2");
    mgr.create_task("task 3");
    let list = mgr.list_tasks();
    assert_eq!(list.len(), 3);
}

#[test]
fn task_stop_sets_cancelled() {
    let mgr = TaskManager::new();
    let id = mgr.create_task("stoppable task");
    mgr.set_status(&id, TaskStatus::Running);
    mgr.stop_task(&id).expect("stop should work");
    assert!(mgr.is_cancelled(&id));
    let info = mgr.get_task(&id).expect("task should exist");
    assert_eq!(info.status, TaskStatus::Stopped);
}

#[test]
fn task_output_empty_initially() {
    let mgr = TaskManager::new();
    let id = mgr.create_task("output task");
    let output = mgr.get_output(&id, None, None).expect("should work");
    assert!(output.is_empty());
}

#[test]
fn task_output_after_append() {
    let mgr = TaskManager::new();
    let id = mgr.create_task("output task");
    mgr.append_output(&id, "hello ");
    mgr.append_output(&id, "world");
    let output = mgr.get_output(&id, None, None).expect("should work");
    assert_eq!(output, "hello world");
}

#[test]
fn task_output_capped_at_1mb() {
    let mgr = TaskManager::new();
    let id = mgr.create_task("big output task");
    // Write 1.5MB worth of data
    let chunk = "x".repeat(500_000);
    mgr.append_output(&id, &chunk);
    mgr.append_output(&id, &chunk);
    mgr.append_output(&id, &chunk);
    let output = mgr.get_output(&id, None, None).expect("should work");
    assert!(
        output.len() <= 1_048_576,
        "output should be capped at 1MB, got {} bytes",
        output.len()
    );
}

#[test]
fn task_output_with_offset_limit() {
    let mgr = TaskManager::new();
    let id = mgr.create_task("offset task");
    mgr.append_output(&id, "0123456789");
    let output = mgr.get_output(&id, Some(3), Some(4)).expect("should work");
    assert_eq!(output, "3456");
}

#[test]
fn task_status_transitions() {
    let mgr = TaskManager::new();
    let id = mgr.create_task("status task");
    assert_eq!(mgr.get_task(&id).unwrap().status, TaskStatus::Pending);

    mgr.set_status(&id, TaskStatus::Running);
    assert_eq!(mgr.get_task(&id).unwrap().status, TaskStatus::Running);

    mgr.set_status(&id, TaskStatus::Completed);
    assert_eq!(mgr.get_task(&id).unwrap().status, TaskStatus::Completed);
    assert!(mgr.get_task(&id).unwrap().completed_at.is_some());
}

#[test]
fn task_status_invalid_transition() {
    let mgr = TaskManager::new();
    let id = mgr.create_task("status task");
    mgr.set_status(&id, TaskStatus::Running);
    mgr.set_status(&id, TaskStatus::Completed);

    // Completed -> Running should be ignored (no panic, status stays Completed)
    mgr.set_status(&id, TaskStatus::Running);
    assert_eq!(mgr.get_task(&id).unwrap().status, TaskStatus::Completed);
}

#[test]
fn task_manager_concurrent_access() {
    let mgr = Arc::new(TaskManager::new());
    let mut handles = Vec::new();

    for i in 0..10 {
        let mgr_clone = Arc::clone(&mgr);
        handles.push(thread::spawn(move || {
            let id = mgr_clone.create_task(&format!("concurrent task {i}"));
            mgr_clone.append_output(&id, &format!("output from {i}"));
            mgr_clone.set_status(&id, TaskStatus::Running);
            id
        }));
    }

    let ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(ids.len(), 10);
    assert_eq!(mgr.list_tasks().len(), 10);
}

// ---------------------------------------------------------------------------
// Tool name tests
// ---------------------------------------------------------------------------

#[test]
fn task_create_tool_name() {
    let tool = TaskCreateTool;
    assert_eq!(tool.name(), "TaskCreate");
}

#[test]
fn task_get_tool_name() {
    let tool = TaskGetTool;
    assert_eq!(tool.name(), "TaskGet");
}

#[test]
fn all_six_tool_names() {
    assert_eq!(TaskCreateTool.name(), "TaskCreate");
    assert_eq!(TaskGetTool.name(), "TaskGet");
    assert_eq!(TaskUpdateTool.name(), "TaskUpdate");
    assert_eq!(TaskListTool.name(), "TaskList");
    assert_eq!(TaskStopTool.name(), "TaskStop");
    assert_eq!(TaskOutputTool.name(), "TaskOutput");
}

// ---------------------------------------------------------------------------
// Tool execute integration tests (using global TASK_MANAGER)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_create_tool_returns_id_json() {
    let tool = TaskCreateTool;
    let input = serde_json::json!({
        "subject": "test subject",
        "description": "test description"
    });
    let result = tool.execute(input, &make_ctx()).await;
    assert!(!result.is_error, "unexpected error: {}", result.content);

    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let task_id = parsed["task_id"].as_str().unwrap();
    assert_eq!(task_id.len(), 8);
}

// TASK-AGS-105: The old `task_create_returns_subagent_request` test asserted
// a serialized `subagent_request` field in the response, used by the legacy
// `handle_subagent_result` indirection. That indirection is deleted;
// TaskCreate now routes directly through the installed SubagentExecutor and
// returns either a task_id-only response (manual), a spawn marker
// (background) or the final result (foreground). Exercising the prompt path
// requires an installed executor, which the task_create.rs embedded test
// module covers for the manual-task path without the executor seam. The old
// shape test is therefore removed.

#[tokio::test]
async fn task_create_without_prompt_has_no_subagent_request() {
    let tool = TaskCreateTool;
    let input = serde_json::json!({
        "subject": "manual task",
        "description": "tracked manually"
    });
    let result = tool.execute(input, &make_ctx()).await;
    assert!(!result.is_error, "unexpected error: {}", result.content);

    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let task_id = parsed["task_id"].as_str().unwrap();
    assert_eq!(task_id.len(), 8);
    // Should NOT contain subagent_request when no prompt
    assert!(
        parsed.get("subagent_request").is_none(),
        "no-prompt TaskCreate should not include subagent_request"
    );
}

// TASK-AGS-105: `task_create_subagent_request_roundtrip` previously verified
// that the tool serialized SubagentRequest fields back out via the response
// JSON. With the indirection removed, SubagentRequest parsing is exercised
// at the TaskCreateTool::execute entry point directly and covered by the
// validation tests below (task_create_invalid_max_turns_errors) plus the
// task_create.rs embedded tests. The round-trip assertion no longer has a
// stable shape to target and is removed.

#[tokio::test]
async fn task_create_invalid_max_turns_errors() {
    let tool = TaskCreateTool;

    // max_turns: 0
    let result = tool
        .execute(
            serde_json::json!({
                "subject": "bad turns",
                "description": "test",
                "prompt": "do stuff",
                "max_turns": 0
            }),
            &make_ctx(),
        )
        .await;
    assert!(result.is_error, "max_turns=0 should error");

    // max_turns: 101
    let result = tool
        .execute(
            serde_json::json!({
                "subject": "bad turns",
                "description": "test",
                "prompt": "do stuff",
                "max_turns": 101
            }),
            &make_ctx(),
        )
        .await;
    assert!(result.is_error, "max_turns=101 should error");
}

#[test]
fn task_create_permission_level_depends_on_prompt() {
    let tool = TaskCreateTool;

    // No prompt = Safe
    let no_prompt = serde_json::json!({"subject": "x", "description": "y"});
    assert_eq!(tool.permission_level(&no_prompt), PermissionLevel::Safe);

    // With prompt = Risky
    let with_prompt = serde_json::json!({"subject": "x", "description": "y", "prompt": "do stuff"});
    assert_eq!(tool.permission_level(&with_prompt), PermissionLevel::Risky);

    // Empty prompt = Safe
    let empty_prompt = serde_json::json!({"subject": "x", "description": "y", "prompt": "  "});
    assert_eq!(tool.permission_level(&empty_prompt), PermissionLevel::Safe);
}

#[tokio::test]
async fn task_get_tool_returns_info_json() {
    let tool_create = TaskCreateTool;
    let tool_get = TaskGetTool;
    let ctx = make_ctx();

    let create_result = tool_create
        .execute(
            serde_json::json!({
                "subject": "get test",
                "description": "get test desc"
            }),
            &ctx,
        )
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&create_result.content).unwrap();
    let task_id = parsed["task_id"].as_str().unwrap().to_string();

    let get_result = tool_get
        .execute(serde_json::json!({ "task_id": task_id }), &ctx)
        .await;
    assert!(
        !get_result.is_error,
        "unexpected error: {}",
        get_result.content
    );

    let info: serde_json::Value = serde_json::from_str(&get_result.content).unwrap();
    assert_eq!(info["id"].as_str().unwrap(), task_id);
    assert_eq!(info["status"].as_str().unwrap(), "Pending");
}

#[tokio::test]
async fn task_list_tool_returns_array() {
    let tool = TaskListTool;
    let ctx = make_ctx();
    let result = tool.execute(serde_json::json!({}), &ctx).await;
    assert!(!result.is_error, "unexpected error: {}", result.content);

    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert!(parsed.is_array());
}

#[tokio::test]
async fn task_update_tool_works() {
    let ctx = make_ctx();

    let create_result = TaskCreateTool
        .execute(
            serde_json::json!({
                "subject": "update test",
                "description": "original"
            }),
            &ctx,
        )
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&create_result.content).unwrap();
    let task_id = parsed["task_id"].as_str().unwrap().to_string();

    let update_result = TaskUpdateTool
        .execute(
            serde_json::json!({
                "task_id": task_id,
                "description": "updated"
            }),
            &ctx,
        )
        .await;
    assert!(
        !update_result.is_error,
        "unexpected error: {}",
        update_result.content
    );
}

#[tokio::test]
async fn task_stop_tool_works() {
    let ctx = make_ctx();

    let create_result = TaskCreateTool
        .execute(
            serde_json::json!({
                "subject": "stop test",
                "description": "will be stopped"
            }),
            &ctx,
        )
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&create_result.content).unwrap();
    let task_id = parsed["task_id"].as_str().unwrap().to_string();

    // Set to Running first so stop is valid
    TASK_MANAGER.set_status(&task_id, TaskStatus::Running);

    let stop_result = TaskStopTool
        .execute(serde_json::json!({ "task_id": task_id }), &ctx)
        .await;
    assert!(
        !stop_result.is_error,
        "unexpected error: {}",
        stop_result.content
    );
    assert!(TASK_MANAGER.is_cancelled(&task_id));
}

#[tokio::test]
async fn task_output_tool_works() {
    let ctx = make_ctx();

    let create_result = TaskCreateTool
        .execute(
            serde_json::json!({
                "subject": "output test",
                "description": "will have output"
            }),
            &ctx,
        )
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&create_result.content).unwrap();
    let task_id = parsed["task_id"].as_str().unwrap().to_string();

    TASK_MANAGER.append_output(&task_id, "hello output");

    let output_result = TaskOutputTool
        .execute(serde_json::json!({ "task_id": task_id }), &ctx)
        .await;
    assert!(
        !output_result.is_error,
        "unexpected error: {}",
        output_result.content
    );
    assert!(output_result.content.contains("hello output"));
}

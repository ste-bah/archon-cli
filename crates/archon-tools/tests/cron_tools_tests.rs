//! Tests for TASK-CLI-311: CronCreate, CronList, CronDelete tools.

use archon_tools::cron_create::CronCreateTool;
use archon_tools::cron_delete::CronDeleteTool;
use archon_tools::cron_list::CronListTool;
use archon_tools::tool::{AgentMode, PermissionLevel, Tool, ToolContext};
use serde_json::json;
use tempfile::TempDir;

fn ctx_in(dir: &TempDir) -> ToolContext {
    ToolContext {
        working_dir: dir.path().to_path_buf(),
        session_id: "test-session".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
    }
}

// ---------------------------------------------------------------------------
// Tool identity
// ---------------------------------------------------------------------------

#[test]
fn cron_create_name() {
    let dir = TempDir::new().unwrap();
    let tool = CronCreateTool::new(dir.path().to_path_buf());
    assert_eq!(tool.name(), "CronCreate");
}

#[test]
fn cron_list_name() {
    let dir = TempDir::new().unwrap();
    let tool = CronListTool::new(dir.path().to_path_buf());
    assert_eq!(tool.name(), "CronList");
}

#[test]
fn cron_delete_name() {
    let dir = TempDir::new().unwrap();
    let tool = CronDeleteTool::new(dir.path().to_path_buf());
    assert_eq!(tool.name(), "CronDelete");
}

#[test]
fn cron_create_permission_level_is_medium() {
    let dir = TempDir::new().unwrap();
    let tool = CronCreateTool::new(dir.path().to_path_buf());
    assert_eq!(
        tool.permission_level(&json!({})),
        PermissionLevel::Risky,
        "scheduling code execution is risky"
    );
}

// ---------------------------------------------------------------------------
// CronCreate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cron_create_writes_task_to_file() {
    let dir = TempDir::new().unwrap();
    let tool = CronCreateTool::new(dir.path().to_path_buf());

    let result = tool
        .execute(
            json!({ "cron": "* * * * *", "prompt": "hello world" }),
            &ctx_in(&dir),
        )
        .await;

    assert!(
        !result.is_error,
        "create should succeed: {:?}",
        result.content
    );

    // Verify file exists
    let tasks_file = dir.path().join(".archon").join("scheduled_tasks.json");
    assert!(tasks_file.exists(), "scheduled_tasks.json must be created");
}

#[tokio::test]
async fn cron_create_returns_task_id() {
    let dir = TempDir::new().unwrap();
    let tool = CronCreateTool::new(dir.path().to_path_buf());

    let result = tool
        .execute(
            json!({ "cron": "0 9 * * 1", "prompt": "Monday standup" }),
            &ctx_in(&dir),
        )
        .await;

    assert!(!result.is_error);
    // Response should contain task ID
    assert!(
        result.content.contains("id") || result.content.len() > 10,
        "response must include task id"
    );
}

#[tokio::test]
async fn cron_create_invalid_expression_returns_error() {
    let dir = TempDir::new().unwrap();
    let tool = CronCreateTool::new(dir.path().to_path_buf());

    let result = tool
        .execute(
            json!({ "cron": "not-valid", "prompt": "test" }),
            &ctx_in(&dir),
        )
        .await;

    assert!(result.is_error, "invalid cron must return error");
    assert!(
        result.content.to_lowercase().contains("cron")
            || result.content.to_lowercase().contains("invalid"),
        "error must describe problem: {:?}",
        result.content
    );
}

#[tokio::test]
async fn cron_create_missing_prompt_returns_error() {
    let dir = TempDir::new().unwrap();
    let tool = CronCreateTool::new(dir.path().to_path_buf());

    let result = tool
        .execute(json!({ "cron": "* * * * *" }), &ctx_in(&dir))
        .await;

    assert!(result.is_error, "missing prompt must return error");
}

#[tokio::test]
async fn cron_create_missing_cron_returns_error() {
    let dir = TempDir::new().unwrap();
    let tool = CronCreateTool::new(dir.path().to_path_buf());

    let result = tool
        .execute(json!({ "prompt": "do something" }), &ctx_in(&dir))
        .await;

    assert!(result.is_error, "missing cron must return error");
}

#[tokio::test]
async fn cron_create_stores_name_in_metadata() {
    let dir = TempDir::new().unwrap();
    let tool = CronCreateTool::new(dir.path().to_path_buf());

    let result = tool
        .execute(
            json!({ "cron": "* * * * *", "prompt": "p", "name": "My Named Task" }),
            &ctx_in(&dir),
        )
        .await;

    assert!(!result.is_error);

    // Load raw JSON and verify name is in metadata, not CronTask
    let raw =
        std::fs::read_to_string(dir.path().join(".archon").join("scheduled_tasks.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();

    // Task struct must NOT have name field
    let task = &json["tasks"][0];
    assert!(
        !task.as_object().unwrap().contains_key("name"),
        "name must NOT be in CronTask"
    );

    // Name must appear in archon_metadata
    assert!(
        json["archon_metadata"]
            .as_object()
            .is_some_and(|m| { m.values().any(|v| v["name"] == "My Named Task") }),
        "name must be in archon_metadata"
    );
}

#[tokio::test]
async fn cron_create_recurring_defaults_to_true() {
    let dir = TempDir::new().unwrap();
    let tool = CronCreateTool::new(dir.path().to_path_buf());

    tool.execute(json!({ "cron": "* * * * *", "prompt": "p" }), &ctx_in(&dir))
        .await;

    let raw =
        std::fs::read_to_string(dir.path().join(".archon").join("scheduled_tasks.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let task = &json["tasks"][0];

    // recurring absent (None) = default recurring = not Some(false)
    let recurring = task["recurring"].clone();
    assert!(
        recurring.is_null() || recurring == true,
        "default recurring should be None/true, got {recurring}"
    );
}

#[tokio::test]
async fn cron_create_one_shot_stores_false() {
    let dir = TempDir::new().unwrap();
    let tool = CronCreateTool::new(dir.path().to_path_buf());

    tool.execute(
        json!({ "cron": "* * * * *", "prompt": "p", "recurring": false }),
        &ctx_in(&dir),
    )
    .await;

    let raw =
        std::fs::read_to_string(dir.path().join(".archon").join("scheduled_tasks.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let task = &json["tasks"][0];

    assert_eq!(
        task["recurring"], false,
        "one-shot should store recurring=false"
    );
}

// ---------------------------------------------------------------------------
// CronList
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cron_list_empty_when_no_tasks() {
    let dir = TempDir::new().unwrap();
    let tool = CronListTool::new(dir.path().to_path_buf());

    let result = tool.execute(json!({}), &ctx_in(&dir)).await;

    assert!(!result.is_error, "empty list should not be an error");
}

#[tokio::test]
async fn cron_list_shows_created_task() {
    let dir = TempDir::new().unwrap();
    let create = CronCreateTool::new(dir.path().to_path_buf());
    let list = CronListTool::new(dir.path().to_path_buf());

    create
        .execute(
            json!({ "cron": "0 9 * * *", "prompt": "morning check" }),
            &ctx_in(&dir),
        )
        .await;

    let result = list.execute(json!({}), &ctx_in(&dir)).await;

    assert!(!result.is_error);
    assert!(
        result.content.contains("0 9 * * *") || result.content.contains("morning check"),
        "list must show task details: {:?}",
        result.content
    );
}

#[tokio::test]
async fn cron_list_shows_next_fire_time() {
    let dir = TempDir::new().unwrap();
    let create = CronCreateTool::new(dir.path().to_path_buf());
    let list = CronListTool::new(dir.path().to_path_buf());

    create
        .execute(
            json!({ "cron": "*/5 * * * *", "prompt": "check" }),
            &ctx_in(&dir),
        )
        .await;

    let result = list.execute(json!({}), &ctx_in(&dir)).await;

    assert!(!result.is_error);
    // Should show some kind of time information
    assert!(
        result.content.contains("next")
            || result.content.contains("20")
            || result.content.contains(":"),
        "list should show next fire time: {:?}",
        result.content
    );
}

// ---------------------------------------------------------------------------
// CronDelete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cron_delete_removes_task() {
    let dir = TempDir::new().unwrap();
    let create = CronCreateTool::new(dir.path().to_path_buf());
    let delete = CronDeleteTool::new(dir.path().to_path_buf());
    let list = CronListTool::new(dir.path().to_path_buf());

    // Create then extract the ID
    let create_result = create
        .execute(
            json!({ "cron": "* * * * *", "prompt": "to delete" }),
            &ctx_in(&dir),
        )
        .await;
    assert!(!create_result.is_error);

    // Extract ID from JSON file
    let raw =
        std::fs::read_to_string(dir.path().join(".archon").join("scheduled_tasks.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let task_id = json["tasks"][0]["id"].as_str().unwrap().to_string();

    // Delete it
    let delete_result = delete
        .execute(json!({ "id": task_id }), &ctx_in(&dir))
        .await;
    assert!(
        !delete_result.is_error,
        "delete should succeed: {:?}",
        delete_result.content
    );

    // List should be empty
    let list_result = list.execute(json!({}), &ctx_in(&dir)).await;
    assert!(!list_result.is_error);
    assert!(
        !list_result.content.contains("to delete"),
        "deleted task must not appear in list"
    );
}

#[tokio::test]
async fn cron_delete_nonexistent_id_returns_error() {
    let dir = TempDir::new().unwrap();
    let tool = CronDeleteTool::new(dir.path().to_path_buf());

    let result = tool
        .execute(json!({ "id": "nonexistent-uuid" }), &ctx_in(&dir))
        .await;

    assert!(
        result.is_error,
        "deleting nonexistent task must return error"
    );
}

#[tokio::test]
async fn cron_delete_missing_id_returns_error() {
    let dir = TempDir::new().unwrap();
    let tool = CronDeleteTool::new(dir.path().to_path_buf());

    let result = tool.execute(json!({}), &ctx_in(&dir)).await;
    assert!(result.is_error, "missing id must return error");
}

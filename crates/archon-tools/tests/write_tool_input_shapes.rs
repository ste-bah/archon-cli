use std::fs;
use std::path::PathBuf;

use archon_tools::file_write::WriteTool;
use archon_tools::tool::{AgentMode, Tool, ToolContext};
use serde_json::json;

fn test_ctx() -> ToolContext {
    let dir = std::env::temp_dir()
        .join("archon-write-tool-input-shapes")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("create test dir");
    ToolContext {
        working_dir: dir,
        session_id: "test-session".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

fn cleanup(path: PathBuf) {
    let _ = fs::remove_dir_all(path);
}

#[tokio::test]
async fn write_tool_accepts_output_path_alias() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("alias-output-path.txt");

    let result = WriteTool
        .execute(
            json!({ "output_path": "alias-output-path.txt", "content": "alias" }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(fs::read_to_string(&file).expect("read"), "alias");
    cleanup(ctx.working_dir);
}

#[tokio::test]
async fn write_tool_accepts_common_output_file_aliases() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("alias-output-file.txt");

    let result = WriteTool
        .execute(
            json!({
                "output_file": "alias-output-file.txt",
                "data": "alias data"
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(fs::read_to_string(&file).expect("read"), "alias data");
    cleanup(ctx.working_dir);
}

#[tokio::test]
async fn write_tool_accepts_nested_input_wrapper() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("nested-input.txt");

    let result = WriteTool
        .execute(
            json!({ "input": { "file_path": "nested-input.txt", "content": "nested" } }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(fs::read_to_string(&file).expect("read"), "nested");
    cleanup(ctx.working_dir);
}

#[tokio::test]
async fn write_tool_accepts_arguments_wrapper_with_aliases() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("arguments-wrapper.txt");

    let result = WriteTool
        .execute(
            json!({ "arguments": { "path": "arguments-wrapper.txt", "markdown": "wrapped" } }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(fs::read_to_string(&file).expect("read"), "wrapped");
    cleanup(ctx.working_dir);
}

#[tokio::test]
async fn write_tool_accepts_object_wrapped_string_values() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("object-wrapped.txt");

    let result = WriteTool
        .execute(
            json!({
                "file_path": { "value": "object-wrapped.txt" },
                "content": { "text": "wrapped value" }
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(fs::read_to_string(&file).expect("read"), "wrapped value");
    cleanup(ctx.working_dir);
}

#[tokio::test]
async fn write_tool_accepts_stringified_json_input() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("stringified-json.txt");

    let result = WriteTool
        .execute(
            json!(r#"{"file_path":"stringified-json.txt","content":"stringified"}"#),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(fs::read_to_string(&file).expect("read"), "stringified");
    cleanup(ctx.working_dir);
}

#[tokio::test]
async fn write_tool_accepts_stringified_json_wrapper() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("stringified-wrapper.txt");

    let result = WriteTool
        .execute(
            json!({
                "tool_input": r#"{"path":"stringified-wrapper.txt","markdown":"wrapped"}"#
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(fs::read_to_string(&file).expect("read"), "wrapped");
    cleanup(ctx.working_dir);
}

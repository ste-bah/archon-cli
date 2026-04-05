use std::fs;
use std::path::PathBuf;

use serde_json::json;

use archon_tools::file_edit::EditTool;
use archon_tools::file_read::ReadTool;
use archon_tools::file_write::WriteTool;
use archon_tools::glob_tool::GlobTool;
use archon_tools::grep::GrepTool;
use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

fn test_ctx() -> ToolContext {
    let dir = std::env::temp_dir()
        .join("archon-tool-tests")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("create test dir");
    ToolContext {
        working_dir: dir,
        session_id: "test-session".into(),
        mode: archon_tools::tool::AgentMode::Normal,
    }
}

fn cleanup(ctx: &ToolContext) {
    let _ = fs::remove_dir_all(&ctx.working_dir);
}

// -----------------------------------------------------------------------
// Trait object safety
// -----------------------------------------------------------------------

#[test]
fn tool_trait_is_object_safe() {
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ReadTool),
        Box::new(WriteTool),
        Box::new(EditTool),
        Box::new(GlobTool),
        Box::new(GrepTool),
    ];
    assert_eq!(tools.len(), 5);
    for tool in &tools {
        assert!(!tool.name().is_empty());
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert!(schema.is_object(), "schema should be a JSON object");
    }
}

// -----------------------------------------------------------------------
// ReadTool
// -----------------------------------------------------------------------

#[tokio::test]
async fn read_tool_reads_file_with_line_numbers() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("test.txt");
    fs::write(&file, "line one\nline two\nline three\n").expect("write");

    let result = ReadTool
        .execute(json!({ "file_path": file.to_str().unwrap() }), &ctx)
        .await;

    assert!(!result.is_error, "should succeed: {}", result.content);
    assert!(result.content.contains("1\tline one"));
    assert!(result.content.contains("2\tline two"));
    assert!(result.content.contains("3\tline three"));
    cleanup(&ctx);
}

#[tokio::test]
async fn read_tool_respects_offset_and_limit() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("big.txt");
    let content: String = (1..=100).map(|i| format!("line {i}\n")).collect();
    fs::write(&file, content).expect("write");

    let result = ReadTool
        .execute(
            json!({ "file_path": file.to_str().unwrap(), "offset": 10, "limit": 5 }),
            &ctx,
        )
        .await;

    assert!(!result.is_error);
    assert!(result.content.contains("11\tline 11"));
    assert!(result.content.contains("15\tline 15"));
    assert!(!result.content.contains("16\tline 16"));
    cleanup(&ctx);
}

#[tokio::test]
async fn read_tool_nonexistent_file() {
    let ctx = test_ctx();
    let result = ReadTool
        .execute(json!({ "file_path": "/nonexistent/file.txt" }), &ctx)
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("does not exist"));
    cleanup(&ctx);
}

#[tokio::test]
async fn read_tool_binary_file() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("binary.bin");
    fs::write(&file, b"\x00\x01\x02\x03").expect("write");

    let result = ReadTool
        .execute(json!({ "file_path": file.to_str().unwrap() }), &ctx)
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("binary"));
    cleanup(&ctx);
}

#[tokio::test]
async fn read_tool_missing_file_path() {
    let ctx = test_ctx();
    let result = ReadTool.execute(json!({}), &ctx).await;
    assert!(result.is_error);
    assert!(result.content.contains("file_path"));
    cleanup(&ctx);
}

// -----------------------------------------------------------------------
// WriteTool
// -----------------------------------------------------------------------

#[tokio::test]
async fn write_tool_creates_new_file() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("new.txt");

    let result = WriteTool
        .execute(
            json!({ "file_path": file.to_str().unwrap(), "content": "hello world" }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(fs::read_to_string(&file).expect("read"), "hello world");
    cleanup(&ctx);
}

#[tokio::test]
async fn write_tool_creates_parent_directories() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("deep/nested/dir/file.txt");

    let result = WriteTool
        .execute(
            json!({ "file_path": file.to_str().unwrap(), "content": "nested" }),
            &ctx,
        )
        .await;

    assert!(!result.is_error);
    assert!(file.exists());
    cleanup(&ctx);
}

#[tokio::test]
async fn write_tool_overwrites_existing() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("existing.txt");
    fs::write(&file, "old content").expect("write");

    WriteTool
        .execute(
            json!({ "file_path": file.to_str().unwrap(), "content": "new content" }),
            &ctx,
        )
        .await;

    assert_eq!(fs::read_to_string(&file).expect("read"), "new content");
    cleanup(&ctx);
}

// -----------------------------------------------------------------------
// EditTool
// -----------------------------------------------------------------------

#[tokio::test]
async fn edit_tool_replaces_unique_string() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("edit.txt");
    fs::write(&file, "hello world\ngoodbye world\n").expect("write");

    let result = EditTool
        .execute(
            json!({
                "file_path": file.to_str().unwrap(),
                "old_string": "hello world",
                "new_string": "hi there"
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    let content = fs::read_to_string(&file).expect("read");
    assert!(content.contains("hi there"));
    assert!(content.contains("goodbye world"));
    cleanup(&ctx);
}

#[tokio::test]
async fn edit_tool_fails_on_ambiguous_match() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("ambig.txt");
    fs::write(&file, "foo\nfoo\nbar\n").expect("write");

    let result = EditTool
        .execute(
            json!({
                "file_path": file.to_str().unwrap(),
                "old_string": "foo",
                "new_string": "baz"
            }),
            &ctx,
        )
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("2 locations"));
    cleanup(&ctx);
}

#[tokio::test]
async fn edit_tool_replace_all() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("replall.txt");
    fs::write(&file, "foo\nfoo\nbar\n").expect("write");

    let result = EditTool
        .execute(
            json!({
                "file_path": file.to_str().unwrap(),
                "old_string": "foo",
                "new_string": "baz",
                "replace_all": true
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error);
    let content = fs::read_to_string(&file).expect("read");
    assert_eq!(content, "baz\nbaz\nbar\n");
    cleanup(&ctx);
}

#[tokio::test]
async fn edit_tool_old_string_not_found() {
    let ctx = test_ctx();
    let file = ctx.working_dir.join("nf.txt");
    fs::write(&file, "some content").expect("write");

    let result = EditTool
        .execute(
            json!({
                "file_path": file.to_str().unwrap(),
                "old_string": "nonexistent",
                "new_string": "replacement"
            }),
            &ctx,
        )
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("not found"));
    cleanup(&ctx);
}

// -----------------------------------------------------------------------
// GlobTool
// -----------------------------------------------------------------------

#[tokio::test]
async fn glob_tool_finds_files() {
    let ctx = test_ctx();
    fs::write(ctx.working_dir.join("a.rs"), "").expect("write");
    fs::write(ctx.working_dir.join("b.rs"), "").expect("write");
    fs::write(ctx.working_dir.join("c.txt"), "").expect("write");

    let result = GlobTool.execute(json!({ "pattern": "*.rs" }), &ctx).await;

    assert!(!result.is_error);
    assert!(result.content.contains("a.rs"));
    assert!(result.content.contains("b.rs"));
    assert!(!result.content.contains("c.txt"));
    cleanup(&ctx);
}

#[tokio::test]
async fn glob_tool_no_matches() {
    let ctx = test_ctx();

    let result = GlobTool
        .execute(json!({ "pattern": "*.nonexistent" }), &ctx)
        .await;

    assert!(!result.is_error);
    assert!(result.content.contains("No files matched"));
    cleanup(&ctx);
}

// -----------------------------------------------------------------------
// GrepTool
// -----------------------------------------------------------------------

#[tokio::test]
async fn grep_tool_finds_matches() {
    let ctx = test_ctx();
    fs::write(
        ctx.working_dir.join("src.rs"),
        "fn main() {}\nfn helper() {}\n",
    )
    .expect("write");

    let result = GrepTool
        .execute(
            json!({
                "pattern": "fn.*main",
                "path": ctx.working_dir.to_str().unwrap(),
                "output_mode": "content"
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("fn main"));
    assert!(!result.content.contains("fn helper"));
    cleanup(&ctx);
}

#[tokio::test]
async fn grep_tool_files_with_matches_mode() {
    let ctx = test_ctx();
    fs::write(ctx.working_dir.join("a.txt"), "needle here").expect("write");
    fs::write(ctx.working_dir.join("b.txt"), "no match").expect("write");

    let result = GrepTool
        .execute(
            json!({
                "pattern": "needle",
                "path": ctx.working_dir.to_str().unwrap(),
                "output_mode": "files_with_matches"
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error);
    assert!(result.content.contains("a.txt"));
    assert!(!result.content.contains("b.txt"));
    cleanup(&ctx);
}

#[tokio::test]
async fn grep_tool_count_mode() {
    let ctx = test_ctx();
    fs::write(ctx.working_dir.join("multi.txt"), "foo\nbar\nfoo\n").expect("write");

    let result = GrepTool
        .execute(
            json!({
                "pattern": "foo",
                "path": ctx.working_dir.join("multi.txt").to_str().unwrap(),
                "output_mode": "count"
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error);
    assert!(result.content.contains(":2"));
    cleanup(&ctx);
}

#[tokio::test]
async fn grep_tool_invalid_regex() {
    let ctx = test_ctx();
    let result = GrepTool
        .execute(json!({ "pattern": "[invalid" }), &ctx)
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("Invalid regex"));
    cleanup(&ctx);
}

// -----------------------------------------------------------------------
// Permission levels
// -----------------------------------------------------------------------

#[test]
fn permission_levels_correct() {
    let input = json!({});
    assert_eq!(ReadTool.permission_level(&input), PermissionLevel::Safe);
    assert_eq!(WriteTool.permission_level(&input), PermissionLevel::Risky);
    assert_eq!(EditTool.permission_level(&input), PermissionLevel::Risky);
    assert_eq!(GlobTool.permission_level(&input), PermissionLevel::Safe);
    assert_eq!(GrepTool.permission_level(&input), PermissionLevel::Safe);
}

// -----------------------------------------------------------------------
// Schema validation
// -----------------------------------------------------------------------

#[test]
fn schemas_are_valid_json_objects() {
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ReadTool),
        Box::new(WriteTool),
        Box::new(EditTool),
        Box::new(GlobTool),
        Box::new(GrepTool),
    ];
    for tool in &tools {
        let schema = tool.input_schema();
        assert!(
            schema.get("type").is_some(),
            "{} schema missing 'type'",
            tool.name()
        );
        assert!(
            schema.get("properties").is_some(),
            "{} schema missing 'properties'",
            tool.name()
        );
        assert!(
            schema.get("required").is_some(),
            "{} schema missing 'required'",
            tool.name()
        );
    }
}

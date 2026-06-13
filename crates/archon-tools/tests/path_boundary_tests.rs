use std::fs;
use std::path::PathBuf;

use archon_tools::file_read::ReadTool;
use archon_tools::glob_tool::GlobTool;
use archon_tools::grep::GrepTool;
use archon_tools::tool::{AgentMode, Tool, ToolContext};
use serde_json::json;

fn ctx_in(dir: PathBuf) -> ToolContext {
    ToolContext {
        working_dir: dir,
        session_id: "path-boundary-test".into(),
        mode: AgentMode::Normal,
        extra_dirs: Vec::new(),
        ..Default::default()
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("archon-path-boundary-tests")
        .join(format!("{label}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[tokio::test]
async fn read_rejects_paths_outside_allowed_roots() {
    let allowed = temp_dir("allowed-read");
    let outside = temp_dir("outside-read");
    let outside_file = outside.join("secret.txt");
    fs::write(&outside_file, "secret").expect("write outside file");

    let result = ReadTool
        .execute(
            json!({ "file_path": outside_file.to_str().unwrap() }),
            &ctx_in(allowed.clone()),
        )
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("outside allowed directories"));
    let _ = fs::remove_dir_all(allowed);
    let _ = fs::remove_dir_all(outside);
}

#[tokio::test]
async fn glob_rejects_paths_outside_allowed_roots() {
    let allowed = temp_dir("allowed-glob");
    let outside = temp_dir("outside-glob");

    let result = GlobTool
        .execute(
            json!({ "path": outside.to_str().unwrap(), "pattern": "*.rs" }),
            &ctx_in(allowed.clone()),
        )
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("outside allowed directories"));
    let _ = fs::remove_dir_all(allowed);
    let _ = fs::remove_dir_all(outside);
}

#[tokio::test]
async fn grep_rejects_paths_outside_allowed_roots() {
    let allowed = temp_dir("allowed-grep");
    let outside = temp_dir("outside-grep");

    let result = GrepTool
        .execute(
            json!({ "path": outside.to_str().unwrap(), "pattern": "anything" }),
            &ctx_in(allowed.clone()),
        )
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("outside allowed directories"));
    let _ = fs::remove_dir_all(allowed);
    let _ = fs::remove_dir_all(outside);
}

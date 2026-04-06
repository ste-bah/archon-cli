use std::fs;

use archon_tools::notebook::NotebookEditTool;
use archon_tools::tool::{AgentMode, Tool, ToolContext};

fn test_notebook_json() -> &'static str {
    r##"{
  "nbformat": 4,
  "nbformat_minor": 5,
  "metadata": {"kernelspec": {"name": "python3"}},
  "cells": [
    {"cell_type": "code", "source": ["print('hello')"], "metadata": {}, "execution_count": 1, "outputs": [{"output_type": "stream", "text": ["hello\n"]}]},
    {"cell_type": "markdown", "source": ["# Title"], "metadata": {}}
  ]
}"##
}

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test-notebook".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
    }
}

fn write_notebook(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("test.ipynb");
    fs::write(&path, test_notebook_json()).unwrap();
    path
}

#[tokio::test]
async fn insert_code_cell_at_beginning() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nb_path = write_notebook(tmp.path());
    let tool = NotebookEditTool;

    let result = tool
        .execute(
            serde_json::json!({
                "path": nb_path.to_str().unwrap(),
                "command": "insert",
                "cell_index": 0,
                "content": "x = 42",
                "cell_type": "code"
            }),
            &make_ctx(),
        )
        .await;

    assert!(
        !result.is_error,
        "insert should succeed: {}",
        result.content
    );

    let nb: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&nb_path).unwrap()).unwrap();
    let cells = nb["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 3);
    assert_eq!(cells[0]["source"][0].as_str().unwrap(), "x = 42");
}

#[tokio::test]
async fn insert_markdown_cell_at_end() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nb_path = write_notebook(tmp.path());
    let tool = NotebookEditTool;

    let result = tool
        .execute(
            serde_json::json!({
                "path": nb_path.to_str().unwrap(),
                "command": "insert",
                "cell_index": 2,
                "content": "## Section",
                "cell_type": "markdown"
            }),
            &make_ctx(),
        )
        .await;

    assert!(
        !result.is_error,
        "insert at end should succeed: {}",
        result.content
    );

    let nb: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&nb_path).unwrap()).unwrap();
    let cells = nb["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 3);
    assert_eq!(cells[2]["cell_type"].as_str().unwrap(), "markdown");
    assert_eq!(cells[2]["source"][0].as_str().unwrap(), "## Section");
}

#[tokio::test]
async fn replace_cell_content() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nb_path = write_notebook(tmp.path());
    let tool = NotebookEditTool;

    let result = tool
        .execute(
            serde_json::json!({
                "path": nb_path.to_str().unwrap(),
                "command": "replace",
                "cell_index": 0,
                "content": "print('world')"
            }),
            &make_ctx(),
        )
        .await;

    assert!(
        !result.is_error,
        "replace should succeed: {}",
        result.content
    );

    let nb: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&nb_path).unwrap()).unwrap();
    let cells = nb["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0]["source"][0].as_str().unwrap(), "print('world')");
}

#[tokio::test]
async fn delete_cell() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nb_path = write_notebook(tmp.path());
    let tool = NotebookEditTool;

    let result = tool
        .execute(
            serde_json::json!({
                "path": nb_path.to_str().unwrap(),
                "command": "delete",
                "cell_index": 1
            }),
            &make_ctx(),
        )
        .await;

    assert!(
        !result.is_error,
        "delete should succeed: {}",
        result.content
    );

    let nb: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&nb_path).unwrap()).unwrap();
    let cells = nb["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0]["cell_type"].as_str().unwrap(), "code");
}

#[tokio::test]
async fn move_cell() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nb_path = write_notebook(tmp.path());
    let tool = NotebookEditTool;

    let result = tool
        .execute(
            serde_json::json!({
                "path": nb_path.to_str().unwrap(),
                "command": "move",
                "cell_index": 0,
                "target_index": 1
            }),
            &make_ctx(),
        )
        .await;

    assert!(!result.is_error, "move should succeed: {}", result.content);

    let nb: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&nb_path).unwrap()).unwrap();
    let cells = nb["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 2);
    // After moving cell 0 to index 1, the markdown cell should be first
    assert_eq!(cells[0]["cell_type"].as_str().unwrap(), "markdown");
    assert_eq!(cells[1]["cell_type"].as_str().unwrap(), "code");
}

#[tokio::test]
async fn preserve_metadata_on_edit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nb_path = write_notebook(tmp.path());
    let tool = NotebookEditTool;

    // Edit cell 0, verify cell 1 metadata is unchanged
    tool.execute(
        serde_json::json!({
            "path": nb_path.to_str().unwrap(),
            "command": "replace",
            "cell_index": 0,
            "content": "new code"
        }),
        &make_ctx(),
    )
    .await;

    let nb: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&nb_path).unwrap()).unwrap();
    let cells = nb["cells"].as_array().unwrap();
    assert_eq!(cells[1]["cell_type"].as_str().unwrap(), "markdown");
    assert_eq!(cells[1]["source"][0].as_str().unwrap(), "# Title");
}

#[tokio::test]
async fn preserve_outputs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nb_path = write_notebook(tmp.path());
    let tool = NotebookEditTool;

    // Edit cell 1 (markdown), verify cell 0 outputs preserved
    tool.execute(
        serde_json::json!({
            "path": nb_path.to_str().unwrap(),
            "command": "replace",
            "cell_index": 1,
            "content": "## New Title"
        }),
        &make_ctx(),
    )
    .await;

    let nb: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&nb_path).unwrap()).unwrap();
    let cells = nb["cells"].as_array().unwrap();
    let outputs = cells[0]["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0]["output_type"].as_str().unwrap(), "stream");
}

#[tokio::test]
async fn invalid_index_returns_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nb_path = write_notebook(tmp.path());
    let tool = NotebookEditTool;

    let result = tool
        .execute(
            serde_json::json!({
                "path": nb_path.to_str().unwrap(),
                "command": "delete",
                "cell_index": 99
            }),
            &make_ctx(),
        )
        .await;

    assert!(result.is_error, "should error on invalid index");
}

#[tokio::test]
async fn non_ipynb_returns_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let py_path = tmp.path().join("script.py");
    fs::write(&py_path, "print('hi')").unwrap();
    let tool = NotebookEditTool;

    let result = tool
        .execute(
            serde_json::json!({
                "path": py_path.to_str().unwrap(),
                "command": "delete",
                "cell_index": 0
            }),
            &make_ctx(),
        )
        .await;

    assert!(result.is_error, "should error on non-ipynb file");
}

#[tokio::test]
async fn malformed_json_returns_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nb_path = tmp.path().join("bad.ipynb");
    fs::write(&nb_path, "this is not json {{{").unwrap();
    let tool = NotebookEditTool;

    let result = tool
        .execute(
            serde_json::json!({
                "path": nb_path.to_str().unwrap(),
                "command": "delete",
                "cell_index": 0
            }),
            &make_ctx(),
        )
        .await;

    assert!(result.is_error, "should error on malformed json");
}

#[test]
fn name_is_notebook_edit() {
    let tool = NotebookEditTool;
    assert_eq!(tool.name(), "NotebookEdit");
}

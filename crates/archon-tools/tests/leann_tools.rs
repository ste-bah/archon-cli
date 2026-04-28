//! Tests for LEANN semantic search tools (Deliverable C, v0.1.21).
//!
//! These test metadata, validation, and permission level without requiring
//! a real CozoDB-backed CodeIndex. Integration tests that exercise actual
//! search/find_similar against an indexed repo live in archon-leann's own
//! test suite.

use std::sync::Arc;

use serde_json::json;

use archon_leann::CodeIndex;
use archon_tools::leann_find_similar::LeannFindSimilarTool;
use archon_tools::leann_search::LeannSearchTool;
use archon_tools::tool::{PermissionLevel, Tool, ToolContext};

fn temp_index() -> Arc<CodeIndex> {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test_leann.db");
    let idx = CodeIndex::new(db_path, Default::default())
        .expect("CodeIndex::new with empty temp dir should succeed");
    Arc::new(idx)
}

fn test_ctx() -> ToolContext {
    ToolContext::default()
}

// ── LeannSearchTool ─────────────────────────────────────────────────

#[test]
fn leann_search_name_and_description() {
    let tool = LeannSearchTool::new(temp_index());
    assert_eq!(tool.name(), "LeannSearch");
    assert!(
        tool.description()
            .to_lowercase()
            .contains("semantic search")
    );
}

#[tokio::test]
async fn leann_search_validates_query_required() {
    let tool = LeannSearchTool::new(temp_index());
    let result = tool.execute(json!({}), &test_ctx()).await;
    assert!(result.is_error);
    assert!(result.content.contains("query is required"));
}

#[test]
fn leann_search_permission_level_is_safe() {
    let tool = LeannSearchTool::new(temp_index());
    assert_eq!(tool.permission_level(&json!({})), PermissionLevel::Safe);
}

#[tokio::test]
async fn leann_search_rejects_empty_query() {
    let tool = LeannSearchTool::new(temp_index());
    let result = tool.execute(json!({"query": ""}), &test_ctx()).await;
    assert!(result.is_error);
}

// ── LeannFindSimilarTool ────────────────────────────────────────────

#[test]
fn leann_find_similar_name_and_description() {
    let tool = LeannFindSimilarTool::new(temp_index());
    assert_eq!(tool.name(), "LeannFindSimilar");
    assert!(tool.description().contains("semantically similar"));
}

#[tokio::test]
async fn leann_find_similar_validates_snippet_required() {
    let tool = LeannFindSimilarTool::new(temp_index());
    let result = tool.execute(json!({}), &test_ctx()).await;
    assert!(result.is_error);
    assert!(result.content.contains("snippet is required"));
}

#[test]
fn leann_find_similar_permission_level_is_safe() {
    let tool = LeannFindSimilarTool::new(temp_index());
    assert_eq!(tool.permission_level(&json!({})), PermissionLevel::Safe);
}

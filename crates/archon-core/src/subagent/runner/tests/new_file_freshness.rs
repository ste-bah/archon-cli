//! Creation must work through the actual runner, not by bypassing it with Bash.
use super::*;

async fn write_through_runner(existing: bool) -> (tempfile::TempDir, Vec<LlmRequest>) {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("new.txt");
    if existing { std::fs::write(&file, "original").unwrap(); }
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("write-new", "Write", &serde_json::json!({
            "file_path":file,"content":"created by Write"
        }).to_string()),
        text_response("done"),
    ]));
    let mut config = AgentConfig::default();
    config.filesystem.read_before_edit = crate::config::ReadBeforeEdit::Block;
    let mut runner = make_runner_with_config(provider.clone(), 2, config);
    runner.tool_context.working_dir = temp.path().to_path_buf();
    runner.tool_context.session_id = uuid::Uuid::new_v4().to_string();
    runner.tool_context.workflow_read_guard = Some(Arc::new(
        archon_tools::workflow_read_guard::WorkflowReadGuard::new(0, 20, false, false),
    ));
    runner.run("Write the deliverable").await.unwrap();
    (temp, provider.requests())
}

#[tokio::test]
async fn freshness_allows_new_file_without_impossible_prior_read() {
    let (temp, requests) = write_through_runner(false).await;
    assert_eq!(std::fs::read_to_string(temp.path().join("new.txt")).unwrap(), "created by Write");
    assert_eq!(requests[1].messages.last().unwrap()["content"][0]["is_error"], false);
}

#[tokio::test]
async fn freshness_still_refuses_unread_existing_file() {
    let (temp, requests) = write_through_runner(true).await;
    assert_eq!(std::fs::read_to_string(temp.path().join("new.txt")).unwrap(), "original");
    assert_eq!(requests[1].messages.last().unwrap()["content"][0]["is_error"], true);
}

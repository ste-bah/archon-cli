//! The host tells a write agent to submit once its declared focused tests have
//! all exited 0, as one user turn the next model request sees.
use super::*;

#[tokio::test]
async fn declared_tests_passing_puts_the_submit_instruction_in_the_next_request() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("docs")).unwrap();
    std::fs::write(temp.path().join("docs/y.md"), "not empty\n").unwrap();
    let declared = "test -s docs/y.md";
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("run-1", "Bash", &serde_json::json!({"command": declared}).to_string()),
        text_response("{\"status\":\"accepted\"}"),
    ]));
    let mut runner = make_runner(provider.clone(), 3);
    runner.tool_context.working_dir = temp.path().to_path_buf();
    runner.tool_context.session_id = uuid::Uuid::new_v4().to_string();
    runner.tool_context.workflow_read_guard = Some(Arc::new(
        archon_tools::workflow_read_guard::WorkflowReadGuard::new(40, 20, false, false)
            .with_focused_tests(archon_tools::workflow_read_guard::FocusedTestPlan::new(
                vec![declared.to_string()],
                15,
            )),
    ));
    runner.run("Implement the task").await.unwrap();
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let messages = &requests[1].messages;
    let last = messages.last().unwrap();
    assert_eq!(last["role"], "user");
    let text = last["content"].as_str().unwrap_or_default();
    assert!(
        text.contains("All declared focused tests have passed in this session (1 of 1 at tool call 1)"),
        "{last}"
    );
    assert!(text.contains("Return the result envelope now."), "{text}");
    // The tool result itself precedes it, unchanged.
    let tool_result = &messages[messages.len() - 2];
    assert_eq!(tool_result["content"][0]["is_error"], false, "{tool_result}");
}

#[tokio::test]
async fn a_task_declaring_no_tests_gets_no_instruction() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("y.md"), "not empty\n").unwrap();
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("run-1", "Bash", &serde_json::json!({"command": "test -s y.md"}).to_string()),
        text_response("{\"status\":\"accepted\"}"),
    ]));
    let mut runner = make_runner(provider.clone(), 3);
    runner.tool_context.working_dir = temp.path().to_path_buf();
    runner.tool_context.session_id = uuid::Uuid::new_v4().to_string();
    runner.tool_context.workflow_read_guard = Some(Arc::new(
        archon_tools::workflow_read_guard::WorkflowReadGuard::new(40, 20, false, false),
    ));
    runner.run("Implement the task").await.unwrap();
    let requests = provider.requests();
    let text = serde_json::to_string(&requests[1].messages).unwrap();
    assert!(!text.contains("All declared focused tests have passed"), "{text}");
}

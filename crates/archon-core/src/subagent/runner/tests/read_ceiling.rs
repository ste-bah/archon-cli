use super::*;

fn ceiling_runner(provider: Arc<MockProvider>) -> SubagentRunner {
    let mut runner = make_runner(provider, 5);
    let settings = archon_tools::workflow_read_guard::WorkflowReadGuardSettings {
        read_only_soft_call_ceiling: 0,
        read_only_hard_call_ceiling: 1,
        ..Default::default()
    };
    runner.tool_context.workflow_read_guard = Some(Arc::new(
        archon_tools::workflow_read_guard::WorkflowReadGuard::shell_only(&settings),
    ));
    runner
}

fn names(request: &LlmRequest) -> Vec<&str> {
    request
        .tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(serde_json::Value::as_str))
        .collect()
}

#[tokio::test]
async fn ceiling_withdraws_inspection_tools_and_runner_returns_the_answer() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("read-1", "Read", r#"{"file_path":"Cargo.toml"}"#),
        text_response("deliverable from the inspected file"),
    ]));
    let runner = ceiling_runner(provider.clone());

    assert_eq!(
        runner.run("inspect then answer").await.unwrap(),
        "deliverable from the inspected file"
    );

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(names(&requests[0]).contains(&"Read"));
    let offered = names(&requests[1]);
    let settings = archon_tools::workflow_read_guard::WorkflowReadGuardSettings {
        read_only_soft_call_ceiling: 0,
        read_only_hard_call_ceiling: 1,
        ..Default::default()
    };
    let guard = archon_tools::workflow_read_guard::WorkflowReadGuard::shell_only(&settings);
    assert_eq!(guard.before_tool("Read", &serde_json::json!({})), None);
    for &tool in archon_tools::workflow_read_guard::READ_ONLY_INSPECTION_TOOLS {
        assert!(!offered.contains(&tool), "{tool} remains offered");
        assert!(
            guard.before_tool(tool, &serde_json::json!({})).is_some(),
            "{tool} remains admitted past the hard ceiling"
        );
    }
    assert!(offered.contains(&"Bash"), "shell must remain offered");
    let next_messages = serde_json::to_string(&requests[1].messages).unwrap();
    assert!(next_messages.contains(
        "Inspection tools are no longer available in this call; answer now with your deliverable from what you have read."
    ));
}

#[tokio::test]
async fn calls_without_the_read_only_guard_keep_the_same_tools() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("read-1", "Read", r#"{"file_path":"Cargo.toml"}"#),
        text_response("answer"),
    ]));
    let runner = make_runner(provider.clone(), 5);

    runner.run("inspect then answer").await.unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(names(&requests[0]), names(&requests[1]));
}

#[tokio::test]
async fn the_request_still_runs_with_no_tools_left_after_filtering() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("read-1", "Read", r#"{"file_path":"Cargo.toml"}"#),
        text_response("answer without tools"),
    ]));
    let mut runner = ceiling_runner(provider.clone());
    runner.tool_definitions = archon_llm::provider::shared_tools(vec![serde_json::json!({
        "name": "Read",
        "description": "Reads a file",
        "input_schema": {"type": "object"}
    })]);

    assert_eq!(
        runner.run("inspect then answer").await.unwrap(),
        "answer without tools"
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].tools.is_empty());
}

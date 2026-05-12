use super::*;

#[tokio::test]
async fn text_only_returns_immediately() {
    let provider = Arc::new(MockProvider::new(vec![text_response(
        "Hello from subagent",
    )]));
    let runner = make_runner(provider.clone(), 10);
    let result = runner.run("Say hello").await.unwrap();
    assert_eq!(result, "Hello from subagent");
    assert_eq!(provider.call_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tool_use_then_text_returns_after_two_turns() {
    let provider = Arc::new(MockProvider::new(vec![
        // Turn 1: LLM uses a tool
        tool_use_response("tool-1", "Read", r#"{"file_path":"/tmp/test.txt"}"#),
        // Turn 2: LLM returns text
        text_response("I read the file."),
    ]));
    let runner = make_runner(provider.clone(), 10);
    let result = runner.run("Read /tmp/test.txt").await.unwrap();
    assert_eq!(result, "I read the file.");
    assert_eq!(provider.call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn max_turns_enforced() {
    // Every turn uses a tool, never returns text
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("t1", "Read", r#"{"file_path":"/tmp/a"}"#),
        tool_use_response("t2", "Read", r#"{"file_path":"/tmp/b"}"#),
        tool_use_response("t3", "Read", r#"{"file_path":"/tmp/c"}"#),
    ]));
    let runner = make_runner(provider.clone(), 2);
    let err = runner.run("keep going").await.unwrap_err();
    assert!(err.to_string().contains("max turns (2)"));
    assert_eq!(provider.call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn api_error_propagated() {
    let provider = Arc::new(MockProvider::new(vec![vec![StreamEvent::Error {
        error_type: "server_error".into(),
        message: "internal failure".into(),
    }]]));
    let runner = make_runner(provider, 10);
    let err = runner.run("trigger error").await.unwrap_err();
    assert!(err.to_string().contains("internal failure"));
}

#[tokio::test]
async fn isolated_messages() {
    // Verify that each run starts with fresh messages
    let provider = Arc::new(MockProvider::new(vec![text_response("First run")]));
    let runner = make_runner(provider.clone(), 10);
    let r1 = runner.run("First prompt").await.unwrap();
    assert_eq!(r1, "First run");

    // Second run should start fresh (provider returns default "(done)")
    let r2 = runner.run("Second prompt").await.unwrap();
    assert_eq!(r2, "(done)");
}

#[tokio::test]
async fn tool_dispatch_error_continues() {
    // Use a nonexistent tool — dispatch should return error, loop continues
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("t1", "NonexistentTool", r#"{}"#),
        text_response("Recovered after tool error"),
    ]));
    let runner = make_runner(provider.clone(), 10);
    let result = runner.run("use bad tool").await.unwrap();
    assert_eq!(result, "Recovered after tool error");
}

#[tokio::test]
async fn empty_tool_definitions_still_works() {
    let provider = Arc::new(MockProvider::new(vec![text_response("No tools needed")]));
    let registry = Arc::new(crate::dispatch::create_default_registry(
        std::env::current_dir().unwrap_or_default(),
        None,
    ));
    let ctx = ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        session_id: "test".into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    };
    let runner = SubagentRunner::new(
        provider,
        "Test agent".into(),
        vec![], // Empty tool defs
        registry,
        ctx,
        "mock".into(),
        5,
        60,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test".into(),
            String::new(),
            String::new(),
        )),
    );
    let result = runner.run("hello").await.unwrap();
    assert_eq!(result, "No tools needed");
}

// snip_context_preserves_recent_turns: removed. archon no longer performs
// client-side context truncation in the subagent runner — context flows to
// the configured LLM provider verbatim, which is the source of truth for
// context limits.

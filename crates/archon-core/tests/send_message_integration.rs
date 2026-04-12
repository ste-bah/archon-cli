//! Integration smoke test: SendMessage tool + SubagentManager name registry + pending messages.
//!
//! Verifies the full flow: tool validates input → name resolves → message queues → drain returns FIFO.

use archon_core::subagent::SubagentManager;
use archon_tools::agent_tool::SubagentRequest;
use archon_tools::send_message::{SendMessageRequest, SendMessageTool};
use archon_tools::tool::{AgentMode, Tool, ToolContext};
use serde_json::json;

fn ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "integration-session".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

fn sample_request() -> SubagentRequest {
    SubagentRequest {
        prompt: "test prompt".into(),
        model: Some("claude-sonnet-4-6".into()),
        allowed_tools: vec![],
        max_turns: 5,
        timeout_secs: 60,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
    }
}

/// End-to-end: SendMessage validates → SubagentManager queues → drain returns messages FIFO.
#[tokio::test]
async fn send_message_to_named_agent_queues_and_drains() {
    // 1. Tool validates input and produces SendMessageRequest
    let tool = SendMessageTool;
    let input = json!({
        "to": "code-reviewer",
        "message": "Review the parser module for correctness",
        "summary": "Review parser module"
    });
    let result = tool.execute(input, &ctx()).await;
    assert!(
        !result.is_error,
        "tool validation failed: {}",
        result.content
    );

    let request: SendMessageRequest = serde_json::from_str(&result.content).expect("valid JSON");
    assert_eq!(request.to, "code-reviewer");

    // 2. SubagentManager resolves name and queues message
    let mut mgr = SubagentManager::new(3);

    // Register an agent and give it a name (simulating Agent tool spawn)
    let agent_id = mgr.register(sample_request()).expect("register succeeds");
    mgr.register_name("code-reviewer".into(), agent_id.clone());

    // Resolve the name (simulating SendMessage intercept in agent.rs)
    let resolved = mgr.resolve_name(&request.to).map(|s| s.to_string());
    assert_eq!(resolved.as_deref(), Some(agent_id.as_str()));

    // Queue the message
    mgr.queue_pending_message(&agent_id, request.message.clone());

    // 3. Drain returns the message (simulating tool round boundary in SubagentRunner)
    let messages = mgr.drain_pending_messages(&agent_id);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0], "Review the parser module for correctness");

    // Second drain is empty (messages consumed)
    let empty = mgr.drain_pending_messages(&agent_id);
    assert!(empty.is_empty(), "drain should be empty after first drain");
}

/// Cleanup removes both name registry and pending messages.
#[tokio::test]
async fn cleanup_agent_removes_name_and_pending() {
    let mut mgr = SubagentManager::new(3);
    let agent_id = mgr.register(sample_request()).expect("register succeeds");
    mgr.register_name("explorer".into(), agent_id.clone());
    mgr.queue_pending_message(&agent_id, "hello".into());

    // Verify they exist
    assert!(mgr.resolve_name("explorer").is_some());

    // Cleanup
    mgr.cleanup_agent(&agent_id);

    // Both gone
    assert!(mgr.resolve_name("explorer").is_none());
    let msgs = mgr.drain_pending_messages(&agent_id);
    assert!(msgs.is_empty());
}

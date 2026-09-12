use super::*;

#[tokio::test]
async fn delivery_parent_next_request_contains_completed_child_report() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("check", "Glob", r#"{"pattern":"no-such-file-*"}"#),
        text_response("use child findings"),
    ]));
    let mut runner = make_runner(provider.clone(), 2);
    let manager = Arc::new(tokio::sync::Mutex::new(crate::subagent::SubagentManager::new(4)));
    let report = format!("{}FINAL_FINDING", "evidence ".repeat(80));
    let envelope = archon_tools::send_message::build_agent_status_envelope(
        "child", Some("reviewer"), archon_tools::send_message::AgentStatusKind::Completed,
        Some(&report),
    );
    manager.lock().await.queue_pending_message("parent", envelope.clone());
    runner.set_pending_message_source(manager, "parent".into());
    runner.run("Use delegated work").await.unwrap();
    assert!(provider.requests()[1].messages.iter().any(|message| message["content"] == envelope));
}

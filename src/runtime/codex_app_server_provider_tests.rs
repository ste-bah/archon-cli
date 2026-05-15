use super::*;

fn configured_provider() -> CodexAppServerProvider {
    CodexAppServerProvider::new(CodexProviderConfig {
        app_server_transport: "stdio".into(),
        app_server_command: "codex".into(),
        ..CodexProviderConfig::default()
    })
    .unwrap()
}

#[tokio::test]
async fn provider_rejects_tools_without_direct_fallback() {
    let provider = configured_provider();
    let request = LlmRequest {
        tools: vec![serde_json::json!({
            "name": "Bash",
            "description": "run command",
            "input_schema": {"type": "object"}
        })],
        ..LlmRequest::default()
    };

    let error = provider.stream(request).await.unwrap_err().to_string();

    assert!(error.contains("cannot execute Archon-managed tool calls directly"));
}

#[test]
fn compaction_summary_uses_logged_generic_fallback() {
    let request = LlmRequest {
        request_origin: Some("compaction_summary".into()),
        ..LlmRequest::default()
    };
    let policy = archon_llm::compaction_policy::compaction_policy_for_family(
        archon_llm::compaction_policy::ProviderFamily::CodexAppServer,
    );

    assert!(log_compaction_fallback_if_needed(&request));
    assert_eq!(
        policy.backend,
        archon_llm::compaction_policy::CompactionBackend::Unsupported
    );
    assert!(policy.generic_fallback);
}

#[test]
fn non_compaction_turn_does_not_emit_fallback_notice() {
    let request = LlmRequest {
        request_origin: Some("main_session".into()),
        ..LlmRequest::default()
    };

    assert!(!log_compaction_fallback_if_needed(&request));
}

#[tokio::test]
async fn projector_emits_completed_turn_text() {
    let (tx, mut rx) = mpsc::channel(16);
    let mut projector =
        AppServerStreamProjector::new("thread-1".into(), "turn-1".into(), "gpt-5.4".into(), tx);
    projector
        .handle_notification(CodexNotification {
            method: "turn/completed".into(),
            params: serde_json::json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "turn": {
                    "id": "turn-1",
                    "threadId": "thread-1",
                    "status": "completed",
                    "items": [{
                        "id": "item-1",
                        "type": "agentMessage",
                        "text": "done"
                    }]
                }
            }),
        })
        .await;

    let mut saw_text = false;
    let mut saw_stop = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            StreamEvent::TextDelta { text, .. } if text == "done" => saw_text = true,
            StreamEvent::MessageStop => saw_stop = true,
            _ => {}
        }
    }

    assert!(saw_text);
    assert!(saw_stop);
    assert!(projector.completed);
}

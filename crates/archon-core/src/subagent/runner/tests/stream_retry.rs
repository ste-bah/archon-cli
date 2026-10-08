use super::*;

fn abandoned() -> Vec<StreamEvent> {
    vec![StreamEvent::MessageStart {
        id: "dropped".into(),
        model: "mock".into(),
        usage: Usage::default(),
    }]
}

#[tokio::test]
async fn dropped_stream_retries_same_history_without_reexploration() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("read-1", "Read", r#"{"file_path":"Cargo.toml","limit":2}"#),
        abandoned(),
        text_response("done"),
    ]));
    let runner = make_runner(provider.clone(), 3);
    assert_eq!(runner.run("Inspect then answer").await.unwrap(), "done");
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[1].messages, requests[2].messages,
        "retry must preserve exact conversation including tool results"
    );
    assert!(requests[2].messages.iter().any(|m| {
        m["content"]
            .as_array()
            .is_some_and(|a| a.iter().any(|b| b["type"] == "tool_result"))
    }));
}

#[tokio::test]
async fn abandoned_partial_tool_is_not_executed_or_concatenated() {
    let mut partial = tool_use_response("abandoned", "Read", r#"{"file_path":"missing-old"}"#);
    partial.pop();
    let provider = Arc::new(MockProvider::new(vec![
        partial,
        tool_use_response("actual", "Read", r#"{"file_path":"Cargo.toml","limit":1}"#),
        text_response("done"),
    ]));
    let runner = make_runner(provider.clone(), 3);
    assert_eq!(runner.run("Inspect").await.unwrap(), "done");
    let requests = provider.requests();
    assert_eq!(requests[0].messages, requests[1].messages);
    assert!(
        !serde_json::to_string(&requests[2].messages)
            .unwrap()
            .contains("abandoned")
    );
}

/// Issue 364 round 5: the bound is a no-progress window, and its stop is the
/// resumable stall marker, never a plain failure after a fixed count.
#[tokio::test]
async fn dropped_stream_retries_are_bounded_by_a_no_progress_window() {
    let provider = Arc::new(MockProvider::new(vec![abandoned(); 200]));
    let config = AgentConfig {
        subagent_stream_idle_timeout_secs: 1,
        ..AgentConfig::default()
    };
    let error = make_runner_with_config(provider.clone(), 5, config)
        .run("answer")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{error}"
    );
    assert!(error.contains("no complete answer"), "{error}");
    let sent = provider.requests().len();
    assert!(
        sent > 4 && sent < 200,
        "it resent inside the window: {sent}"
    );
}

#[tokio::test]
async fn protocol_transport_error_retries_same_round() {
    for error_type in ["network", "http_error", "protocol"] {
        let provider = Arc::new(MockProvider::new(vec![
            vec![StreamEvent::Error {
                error_type: error_type.into(),
                message: "stream ended before message_stop".into(),
            }],
            text_response("done"),
        ]));
        assert_eq!(
            make_runner(provider.clone(), 1)
                .run("answer")
                .await
                .unwrap(),
            "done"
        );
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].messages, requests[1].messages);
    }
}

#[tokio::test]
async fn empty_terminal_round_retries_but_output_limit_does_not() {
    let provider = Arc::new(MockProvider::new(vec![
        vec![
            StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".into()),
                usage: None,
            },
            StreamEvent::MessageStop,
        ],
        text_response("done"),
    ]));
    assert_eq!(
        make_runner(provider.clone(), 1)
            .run("answer")
            .await
            .unwrap(),
        "done"
    );
    assert_eq!(provider.requests().len(), 2);
    let limited = Arc::new(MockProvider::new(vec![vec![
        StreamEvent::MessageDelta {
            stop_reason: Some("max_tokens".into()),
            usage: None,
        },
        StreamEvent::MessageStop,
    ]]));
    assert!(make_runner(limited.clone(), 1).run("answer").await.is_err());
    assert_eq!(limited.requests().len(), 1);
}

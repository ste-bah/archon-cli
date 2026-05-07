use super::*;

// -----------------------------------------------------------------
// TASK-T3 (G4): SubagentRunner accumulates Usage from a streamed turn
// -----------------------------------------------------------------

#[tokio::test]
async fn runner_accumulates_tokens_from_mock_stream() {
    // Single-turn stream: MessageStart with input/cache tokens, then a
    // text body, then MessageDelta carrying the final output_tokens,
    // then MessageStop.  No tool_use, so the runner returns after one turn.
    let stream_events = vec![
        StreamEvent::MessageStart {
            id: "msg-prog-1".into(),
            model: "mock".into(),
            usage: Usage {
                input_tokens: 100,
                output_tokens: 5,
                cache_creation_input_tokens: 10,
                cache_read_input_tokens: 20,
            },
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
            tool_use_id: None,
            tool_name: None,
        },
        StreamEvent::TextDelta {
            index: 0,
            text: "ok".into(),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: Some(Usage {
                input_tokens: 0,
                output_tokens: 25,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            }),
        },
        StreamEvent::MessageStop,
    ];

    let provider = Arc::new(MockProvider::new(vec![stream_events]));
    let mut runner = make_runner(provider, 5);

    // Wire a fresh ProgressTracker arc into the runner.
    let tracker = std::sync::Arc::new(std::sync::Mutex::new(
        crate::subagent::ProgressTracker::default(),
    ));
    runner.set_progress_tracker(tracker.clone());

    let result = runner.run("hello").await.unwrap();
    assert_eq!(result, "ok");

    let g = tracker.lock().unwrap();
    assert_eq!(g.cumulative_input_tokens, 100);
    // 5 from MessageStart + 25 from MessageDelta
    assert_eq!(g.cumulative_output_tokens, 30);
    assert_eq!(g.cumulative_cache_creation_tokens, 10);
    assert_eq!(g.cumulative_cache_read_tokens, 20);
    // No tool_use blocks were dispatched.
    assert_eq!(g.tool_use_count, 0);
    assert!(g.recent_activities.is_empty());
}

#[tokio::test]
async fn runner_increments_tool_use_count_on_dispatch() {
    // Turn 1: tool_use a (will fail dispatch — fine, counter still bumps)
    // Turn 2: text response, runner returns.
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("call-1", "NonexistentTool", r#"{}"#),
        text_response("done"),
    ]));
    let mut runner = make_runner(provider, 5);

    let tracker = std::sync::Arc::new(std::sync::Mutex::new(
        crate::subagent::ProgressTracker::default(),
    ));
    runner.set_progress_tracker(tracker.clone());

    let result = runner.run("use a tool").await.unwrap();
    assert_eq!(result, "done");

    let g = tracker.lock().unwrap();
    assert_eq!(g.tool_use_count, 1);
    assert_eq!(g.recent_activities.len(), 1);
    assert_eq!(
        g.recent_activities.front().unwrap().tool_name,
        "NonexistentTool"
    );
    // Tokens accumulated across two turns:
    // Turn 1 (tool_use_response): MessageStart input=10, output=20
    // Turn 2 (text_response):     MessageStart input=10, output=5
    assert_eq!(g.cumulative_input_tokens, 20);
    assert_eq!(g.cumulative_output_tokens, 25);
}

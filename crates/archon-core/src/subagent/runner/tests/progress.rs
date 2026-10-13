use super::*;

#[tokio::test]
async fn runner_preserves_provider_stop_reason_on_final_answer() {
    let mut events = text_response("truncated answer");
    events.insert(
        events.len() - 1,
        StreamEvent::MessageDelta {
            stop_reason: Some("max_tokens".into()),
            usage: None,
        },
    );
    let provider = Arc::new(MockProvider::new(vec![events]));
    let runner = make_runner(provider, 5);

    let completion = runner.run_with_completion("respond").await.unwrap();
    assert_eq!(completion.content, "truncated answer");
    assert_eq!(completion.stop_reason.as_deref(), Some("max_tokens"));
}

#[tokio::test]
async fn runner_preserves_missing_provider_stop_reason_as_none() {
    let provider = Arc::new(MockProvider::new(vec![text_response("answer")]));
    let runner = make_runner(provider, 5);

    let completion = runner.run_with_completion("respond").await.unwrap();
    assert_eq!(completion.stop_reason, None);
}

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
                input_tokens_available: true,
                output_tokens_available: true,
                ..Usage::default()
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
                output_tokens_available: true,
                ..Usage::default()
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
    // #424: message_delta usage is cumulative for the message, so the
    // final count is the delta's 25, not 5 (MessageStart) + 25.
    assert_eq!(g.cumulative_output_tokens, 25);
    assert_eq!(g.cumulative_cache_creation_tokens, 10);
    assert_eq!(g.cumulative_cache_read_tokens, 20);
    assert!(g.input_tokens_available);
    assert!(g.output_tokens_available);
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

// #424: a provider that reports usage only in the final message_delta (the
// OpenAI-compatible adapters) still reaches the tracker, input included.
#[tokio::test]
async fn runner_counts_delta_only_usage_once_per_message() {
    let delta_only = |text: &str, input: u64, output: u64| {
        vec![
            StreamEvent::ContentBlockStart {
                index: 0,
                block_type: ContentBlockType::Text,
                tool_use_id: None,
                tool_name: None,
            },
            StreamEvent::TextDelta {
                index: 0,
                text: text.into(),
            },
            StreamEvent::ContentBlockStop { index: 0 },
            StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".into()),
                usage: None,
            },
            StreamEvent::MessageDelta {
                stop_reason: None,
                usage: Some(Usage {
                    input_tokens: input,
                    output_tokens: output,
                    input_tokens_available: true,
                    output_tokens_available: true,
                    ..Usage::default()
                }),
            },
            StreamEvent::MessageStop,
        ]
    };
    let provider = Arc::new(MockProvider::new(vec![delta_only("ok", 40, 7)]));
    let mut runner = make_runner(provider, 5);
    let tracker = std::sync::Arc::new(std::sync::Mutex::new(
        crate::subagent::ProgressTracker::default(),
    ));
    runner.set_progress_tracker(tracker.clone());

    assert_eq!(runner.run("hello").await.unwrap(), "ok");

    let g = tracker.lock().unwrap();
    assert_eq!(g.cumulative_input_tokens, 40);
    assert_eq!(g.cumulative_output_tokens, 7);
    assert!(g.input_tokens_available);
    assert!(g.output_tokens_available);
}

/// Issue 369: one response as an OpenAI-compatible proxy streamed it in a
/// live run (its transport evidence `body_first_500` / `body_last_500`): a
/// placeholder 0 for every count in `message_start`, the real input and output
/// in the final `message_delta`. The frames are verbatim but for the id and
/// the stop reason (`end_turn`, so the runner returns after this one turn); a
/// text block stands in for the cut middle. Parsed by the real SSE parser.
#[tokio::test]
async fn runner_takes_input_from_the_final_delta_after_a_placeholder_start() {
    const CAPTURED: &str = r#"event: message_start
data: {"type": "message_start", "message": {"id": "msg_captured", "type": "message", "role": "assistant", "content": [], "model": "GLM-5.3-Flash-EXL3", "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 0, "output_tokens": 0, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}}}

event: content_block_start
data: {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}

event: content_block_delta
data: {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "ok"}}

event: content_block_stop
data: {"type": "content_block_stop", "index": 0}

event: message_delta
data: {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"input_tokens": 17256, "output_tokens": 27}}

event: message_stop
data: {"type": "message_stop"}
"#;
    let events: Vec<StreamEvent> = archon_llm::streaming::split_sse_lines(CAPTURED)
        .into_iter()
        .map(|(event, data)| archon_llm::streaming::parse_sse_event(event, data).unwrap())
        .collect();
    let provider = Arc::new(MockProvider::new(vec![events]));
    let mut runner = make_runner(provider, 5);
    let tracker = std::sync::Arc::new(std::sync::Mutex::new(
        crate::subagent::ProgressTracker::default(),
    ));
    runner.set_progress_tracker(tracker.clone());

    assert_eq!(runner.run("hello").await.unwrap(), "ok");

    let g = tracker.lock().unwrap();
    assert_eq!(
        g.cumulative_input_tokens, 17256,
        "not the start placeholder 0"
    );
    assert_eq!(g.cumulative_output_tokens, 27);
    assert!(g.input_tokens_available && g.output_tokens_available);
}

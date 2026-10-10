use super::*;

#[tokio::test]
async fn text_form_tool_call_is_retried_through_tool_interface() {
    let malformed =
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/tmp/a</arg_value></tool_call>";
    let provider = Arc::new(MockProvider::new(vec![
        text_response(malformed),
        text_response("The answer is ready."),
    ]));
    let runner = make_runner(provider.clone(), 3);

    assert_eq!(
        runner.run("inspect the file").await.unwrap(),
        "The answer is ready."
    );

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let second_history = serde_json::to_string(&requests[1].messages).unwrap();
    assert!(second_history.contains(malformed));
    assert!(second_history.contains(
        "Your last message wrote a tool call as text. Text tool calls are not run. Call the tool through the tool interface, or give your final answer."
    ));
}

#[tokio::test]
async fn repeated_text_form_tool_calls_stop_at_max_turns() {
    let malformed = "<tool_call>Read</tool_call>";
    let provider = Arc::new(MockProvider::new(vec![
        text_response(malformed),
        text_response(malformed),
        text_response(malformed),
    ]));
    let runner = make_runner(provider.clone(), 3);

    let error = runner.run("inspect the file").await.unwrap_err();

    assert!(error.to_string().contains("max turns (3)"));
    assert_eq!(provider.call_count.load(Ordering::SeqCst), 3);
}

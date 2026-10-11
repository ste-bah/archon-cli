use super::*;

#[tokio::test]
async fn unavailable_text_form_tool_call_lists_offered_tools() {
    let provider = Arc::new(MockProvider::new(vec![
        text_response(
            "<tool_call>Bash<arg_key>command</arg_key><arg_value>pwd</arg_value></tool_call>",
        ),
        text_response("The answer is ready."),
    ]));
    let mut runner = make_runner(provider.clone(), 3);
    runner.tool_definitions = archon_llm::provider::shared_tools(vec![
        serde_json::json!({"name":"Read","description":"Reads a file","input_schema":{"type":"object"}}),
        serde_json::json!({"name":"Grep","description":"Searches files","input_schema":{"type":"object"}}),
    ]);

    assert_eq!(
        runner.run("inspect the file").await.unwrap(),
        "The answer is ready."
    );

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let second_history = serde_json::to_string(&requests[1].messages).unwrap();
    assert!(second_history.contains(
        "Your last message began with a call to `Bash` written as text; `Bash` is not available in this call, so it was not run. The tools you can call are: Read, Grep. Call one of them through the tool interface, or, if none fits, answer now with your deliverable from what you have read."
    ));
}

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
        "Your last message began with a tool call written as text; it was not run. If that message also contained your final answer, repeat the final answer now without the tool-call text. Otherwise call the tool through the tool interface."
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

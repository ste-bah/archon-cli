use super::*;

#[tokio::test]
async fn completed_history_captures_tool_results_and_previous_answer_before_validation() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("read-1", "missing-test-tool", "{}"),
        text_response("invalid final envelope"),
        text_response("corrected final envelope"),
    ]));
    let history = archon_tools::subagent_session::CompletedHistory::default();
    let mut first = make_runner(provider.clone(), 5);
    first.set_completed_history(history.clone());
    assert_eq!(
        first.run("original task").await.unwrap(),
        "invalid final envelope"
    );
    let captured = history.messages();
    assert_eq!(captured.len(), 4);
    assert_eq!(
        captured.last().unwrap()["content"],
        "invalid final envelope"
    );
    assert_eq!(captured[2]["content"][0]["type"], "tool_result");
    let mut resumed = make_runner(provider.clone(), 5);
    resumed.set_initial_messages(captured.clone());
    resumed.set_completed_history(history.clone());
    assert_eq!(
        resumed.run("validation feedback").await.unwrap(),
        "corrected final envelope"
    );
    let requests = provider.requests();
    let repair = requests.last().unwrap();
    let text = serde_json::to_string(&repair.messages).unwrap();
    for expected in [
        "original task",
        "read-1",
        "invalid final envelope",
        "validation feedback",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert_eq!(history.messages().len(), 6);
}

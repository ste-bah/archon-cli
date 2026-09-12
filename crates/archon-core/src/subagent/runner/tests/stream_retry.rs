use super::*;

fn abandoned() -> Vec<StreamEvent> {
    vec![StreamEvent::MessageStart {id:"dropped".into(),model:"mock".into(),usage:Usage::default()}]
}

#[tokio::test]
async fn dropped_stream_retries_same_history_without_reexploration() {
    let provider=Arc::new(MockProvider::new(vec![
        tool_use_response("read-1","Read",r#"{"file_path":"Cargo.toml","limit":2}"#),
        abandoned(), text_response("done"),
    ]));
    let runner=make_runner(provider.clone(),3);
    assert_eq!(runner.run("Inspect then answer").await.unwrap(),"done");
    let requests=provider.requests();assert_eq!(requests.len(),3);
    assert_eq!(requests[1].messages, requests[2].messages, "retry must preserve exact conversation including tool results");
    assert!(requests[2].messages.iter().any(|m|m["content"].as_array().is_some_and(|a|a.iter().any(|b|b["type"]=="tool_result"))));
}

#[tokio::test]
async fn abandoned_partial_tool_is_not_executed_or_concatenated() {
    let mut partial=tool_use_response("abandoned","Read",r#"{"file_path":"missing-old"}"#);
    partial.pop();
    let provider=Arc::new(MockProvider::new(vec![partial,
        tool_use_response("actual","Read",r#"{"file_path":"Cargo.toml","limit":1}"#),text_response("done")]));
    let runner=make_runner(provider.clone(),3);
    assert_eq!(runner.run("Inspect").await.unwrap(),"done");
    let requests=provider.requests();
    assert_eq!(requests[0].messages,requests[1].messages);
    assert!(!serde_json::to_string(&requests[2].messages).unwrap().contains("abandoned"));
}

#[tokio::test]
async fn dropped_stream_retries_are_bounded() {
    let provider=Arc::new(MockProvider::new(vec![abandoned(),abandoned(),abandoned(),abandoned()]));
    let error=make_runner(provider.clone(),5).run("answer").await.unwrap_err();
    assert!(error.to_string().contains("stream retry exhausted"),"{error}");
    assert_eq!(provider.requests().len(),4);
}

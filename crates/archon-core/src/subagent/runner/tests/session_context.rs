use super::*;

#[tokio::test]
async fn validation_continuation_preserves_original_system_model_and_effort() {
    let provider = Arc::new(MockProvider::new(vec![
        text_response("first"),
        text_response("second"),
    ]));
    let history = archon_tools::subagent_session::CompletedHistory::default();
    let mut first = make_runner(provider.clone(), 2);
    first.system_prompt = "original system".into();
    first.model = "original-model".into();
    first.set_effort("low".into());
    first
        .preserve_session_context(&history, false)
        .await
        .unwrap();
    first.run("first task").await.unwrap();
    let mut second = make_runner(provider.clone(), 2);
    second.system_prompt = "changed recall and agent definition".into();
    second.model = "changed-model".into();
    second.set_effort("max".into());
    second
        .preserve_session_context(&history, true)
        .await
        .unwrap();
    second.run("repair feedback").await.unwrap();
    let requests = provider.requests();
    assert_eq!(requests[0].system, requests[1].system);
    assert_eq!(requests[0].model, requests[1].model);
    assert_eq!(requests[0].effort, requests[1].effort);
    assert_eq!(requests[1].effort.as_deref(), Some("low"));
}

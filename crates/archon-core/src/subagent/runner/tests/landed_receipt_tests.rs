use super::*;

#[tokio::test]
async fn landed_embedded_receipt_checks_this_calls_count() {
    for (text, requests) in [
        ("Finished. {\"records_landed\":1}", 1),
        ("Finished tasks [1, 2]. {\"records_landed\":9}", 2),
    ] {
        let provider = Arc::new(MockProvider::new(vec![
            text_response(text),
            text_response(r#"{"records_landed":1}"#),
        ]));
        let runner = runner_with(provider.clone(), retained_host(), 4);
        assert_eq!(
            runner.run("continue").await.unwrap(),
            r#"{"records_landed":1}"#
        );
        assert_eq!(provider.requests().len(), requests);
    }
}

#[tokio::test]
async fn landed_incidental_brackets_request_receipt_without_json_error() {
    let provider = Arc::new(MockProvider::new(vec![
        text_response(
            "tasks [A, B] and [1, 2]"
        );
        3
    ]));
    let runner = runner_with(provider.clone(), Arc::new(Host::default()), 4);
    assert!(
        runner
            .run("continue")
            .await
            .unwrap_err()
            .to_string()
            .contains("landing receipt")
    );
    let messages = serde_json::to_string(&provider.requests()[1].messages).unwrap();
    assert!(!messages.contains("malformed JSON"), "{messages}");
    assert!(messages.contains("records_landed"), "{messages}");
}

#[tokio::test(start_paused = true)]
async fn landed_recovery_transport_stall_completes_from_store() {
    // No terminal framing: the resend budget, rather than the final-reply judge, stops this.
    let provider = Arc::new(MockProvider::new(vec![vec![]; 32]));
    let mut runner = runner_with(provider.clone(), retained_host(), 4);
    runner.timeout_secs = 1;
    Arc::make_mut(&mut runner.agent_config).subagent_stream_idle_timeout_secs = 1;
    let answer = archon_tools::provider_recovery::scope(runner.run("continue"))
        .await
        .unwrap();
    assert_eq!(answer, r#"{"records_landed":1}"#);
    assert!(provider.requests().len() > 3);
}

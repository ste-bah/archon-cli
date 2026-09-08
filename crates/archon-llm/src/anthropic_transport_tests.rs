use super::*;

#[tokio::test]
async fn transport_evidence_records_http_and_terminal_reason() {
    use archon_observability::transport::EvidenceScope;
    let server = MockServer::start().await;
    let body = "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":19}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    Mock::given(method("POST")).and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-request-id", "request-1")
            .insert_header("set-cookie", "session=private-cookie").set_body_string(body))
        .mount(&server).await;
    let root = tempfile::tempdir().unwrap();
    let scope = EvidenceScope::new(root.path().join("transport.jsonl"), "branch").unwrap();
    let client = AnthropicClient::new(make_auth(), make_identity(), Some(format!("{}/v1/messages", server.uri())));
    scope.run(async {
        let mut rx = client.stream_message(MessageRequest::default()).await.unwrap();
        while rx.recv().await.is_some() {}
    }).await;
    let raw = std::fs::read_to_string(root.path().join("transport.jsonl")).unwrap();
    let records: Vec<serde_json::Value> = raw.lines().map(|s| serde_json::from_str(s).unwrap()).collect();
    let last = records.last().unwrap();
    assert_eq!(last["http_status"], 200);
    assert_eq!(last["finish_reason"], "max_tokens");
    assert_eq!(last["body_bytes"], body.len());
    assert_eq!(last["body_first_500"], body);
    assert_eq!(last["body_last_500"], body);
    assert_eq!(last["headers"]["x-request-id"], "request-1");
    assert!(!raw.contains("private-cookie"));
}

#[tokio::test]
async fn transport_evidence_empty_http_body_is_protocol_error() {
    use archon_observability::transport::EvidenceScope;
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
    let root = tempfile::tempdir().unwrap();
    let scope = EvidenceScope::new(root.path().join("transport.jsonl"), "empty").unwrap();
    let client = AnthropicClient::new(make_auth(), make_identity(), Some(server.uri()));
    scope.run(async {
        let mut rx = client.stream_message(MessageRequest::default()).await.unwrap();
        assert!(matches!(rx.recv().await, Some(crate::streaming::StreamEvent::Error { error_type, .. }) if error_type == "protocol"));
        assert!(rx.recv().await.is_none());
    }).await;
    let raw = std::fs::read_to_string(root.path().join("transport.jsonl")).unwrap();
    let last: serde_json::Value = serde_json::from_str(raw.lines().last().unwrap()).unwrap();
    assert_eq!(last["http_status"], 200);
    assert_eq!(last["body_bytes"], 0);
    assert_eq!(last["terminal_marker"], false);
    assert_eq!(last["stream_end"], "eof");
}

#[tokio::test]
async fn transport_evidence_carries_rejected_and_malformed_response_bodies() {
    use archon_observability::transport::EvidenceScope;
    for (status, body) in [
        (400, r#"{"error":{"message":"tool input rejected"},"token":"private-token"}"#),
        (200, "event: content_block_delta\ndata: {broken-tool-json}\n\n"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(status).set_body_string(body)).mount(&server).await;
        let root = tempfile::tempdir().unwrap();
        let scope = EvidenceScope::new(root.path().join("transport.jsonl"), "rejected").unwrap();
        let client = AnthropicClient::new(make_auth(), make_identity(), Some(server.uri()));
        scope.run(async {
            match client.stream_message(MessageRequest::default()).await {
                Err(_) => assert_eq!(status, 400),
                Ok(mut rx) => {
                    let mut failed = false;
                    while let Some(event) = rx.recv().await {
                        failed |= matches!(event, crate::streaming::StreamEvent::Error { .. });
                    }
                    assert!(failed, "malformed tool chunk must not become success");
                }
            }
        }).await;
        let raw = std::fs::read_to_string(root.path().join("transport.jsonl")).unwrap();
        let last: serde_json::Value = serde_json::from_str(raw.lines().last().unwrap()).unwrap();
        assert_eq!(last["http_status"], status);
        assert_eq!(last["body_bytes"], body.len());
        assert!(!raw.contains("private-token"));
        assert!(last["body_first_500"].as_str().unwrap().contains(if status == 400 {"tool input rejected"} else {"broken-tool-json"}));
    }
}

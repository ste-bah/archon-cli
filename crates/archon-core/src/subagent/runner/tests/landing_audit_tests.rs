//! Regression shapes and cuts from audit-landing/A-runner.md, D1–D7.
use super::*;

fn retained_host() -> Arc<Host> {
    let host = Arc::new(Host::default());
    host.land(serde_json::from_str(&record("UNIT-1")).unwrap())
        .unwrap();
    host
}

#[tokio::test]
async fn landed_a1_last_tool_turn_completes_from_store() {
    // Report matrix #9: a tool-only final turn, with a landed record.
    let provider = Arc::new(MockProvider::new(vec![tool_use_response(
        "land",
        "land-skeleton-record",
        &record("UNIT-1"),
    )]));
    let host = Arc::new(Host::default());
    let mut runner = runner_with(provider, host.clone(), 1);
    let mut registry = crate::dispatch::ToolRegistry::new();
    registry.register(Box::new(archon_tools::audit_landing::ScopedLandingTool(
        runner.tool_context.audit_landing.clone().unwrap(),
    )));
    runner.tool_definitions = archon_llm::provider::shared_tools(registry.tool_definitions());
    runner.registry = Arc::new(registry);
    let completion = runner.run_with_completion("continue").await.unwrap();
    assert_eq!(host.count(), 1);
    assert_eq!(completion.content, r#"{"records_landed":1}"#);
    assert_eq!(completion.stop_reason.as_deref(), Some("end_turn"));
}

#[tokio::test(start_paused = true)]
async fn landed_a1_expired_window_completes_from_store() {
    // Report matrix #14: a no-progress window cut between turns.
    let provider = Arc::new(MockProvider::new(vec![]));
    let mut runner = runner_with(provider.clone(), retained_host(), 2);
    runner.timeout_secs = 0;
    assert_eq!(
        runner.run("continue").await.unwrap(),
        r#"{"records_landed":1}"#
    );
    assert!(provider.requests().is_empty());
}

#[tokio::test]
async fn landed_a1_shutdown_completes_from_store() {
    let runner = runner_with(Arc::new(MockProvider::new(vec![])), retained_host(), 2);
    runner.shutdown_flag.store(true, Ordering::Relaxed);
    let completion = runner.run_with_completion("continue").await.unwrap();
    assert_eq!(completion.content, r#"{"records_landed":1}"#);
    assert_eq!(completion.stop_reason.as_deref(), Some("shutdown"));
}

#[tokio::test]
async fn landed_a2_record_finals_are_bounded_and_good_landings_complete() {
    // Report D2 / matrix #12: repeated one-record JSON replies.
    let provider = Arc::new(MockProvider::new(
        (1..=3)
            .map(|n| text_response(&record(&format!("UNIT-{n}"))))
            .collect(),
    ));
    let host = Arc::new(Host::default());
    let runner = runner_with(provider.clone(), host.clone(), 9);
    let answer = archon_tools::provider_recovery::scope(runner.run("continue"))
        .await
        .unwrap();
    assert_eq!(provider.requests().len(), 3);
    assert_eq!(host.count(), 3);
    assert_eq!(answer, r#"{"records_landed":3}"#);
}

#[tokio::test]
async fn landed_a3_audit_declared_path_final_is_landed() {
    // Report D3 / matrix #4: audit uses declared_path rather than subject.
    struct AuditHost(Mutex<Vec<Value>>);
    impl LandingHost for AuditHost {
        fn land(&self, value: Value) -> Result<String, String> {
            self.0.lock().unwrap().push(value);
            Ok("record retained".into())
        }
        fn hint(&self) -> Result<String, String> {
            Ok(String::new())
        }
        fn complete(&self, _: &Value) -> Result<(), String> {
            Ok(())
        }
    }
    let host = Arc::new(AuditHost(Mutex::new(vec![])));
    let reply = r#"{"declared_path":"src/lib.rs","verdict":"exists_as_declared","equivalents":[],"required_action":"none","reason":"inspected source"}"#;
    let receipt = r#"{"data":{"repository_audit":{"records_landed":1}}}"#;
    let provider = Arc::new(MockProvider::new(vec![
        text_response(reply),
        text_response(receipt),
    ]));
    let mut runner = make_runner(provider.clone(), 4);
    runner.tool_context.audit_landing = Some(Arc::new(AuditLanding::new(host.clone(), None)));
    assert_eq!(runner.run("inspect").await.unwrap(), receipt);
    assert_eq!(host.0.lock().unwrap().len(), 1);
    assert_eq!(provider.requests().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn landed_a4_empty_end_turn_completes_from_store() {
    let provider = Arc::new(MockProvider::new(vec![vec![
        StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: None,
        },
        StreamEvent::MessageStop,
    ]]));
    let runner = runner_with(provider.clone(), retained_host(), 3);
    assert_eq!(
        runner.run("continue").await.unwrap(),
        r#"{"records_landed":1}"#
    );
    assert_eq!(provider.requests().len(), 1);
}

#[tokio::test]
async fn landed_a5_arrays_and_one_embedded_record_are_retained() {
    // Report D5 / matrix #3b: never discard records in favour of earlier landings.
    for text in [
        format!("[{},{}]", record("UNIT-2"), record("UNIT-3")),
        format!("I inspected this unit: {}. Continue.", record("UNIT-2")),
    ] {
        let expected = if text.starts_with('[') { 3 } else { 2 };
        let receipt = json!({"records_landed": expected}).to_string();
        let provider = Arc::new(MockProvider::new(vec![
            text_response(&text),
            text_response(&receipt),
        ]));
        let host = retained_host();
        let runner = runner_with(provider.clone(), host.clone(), 4);
        assert_eq!(runner.run("continue").await.unwrap(), receipt);
        assert_eq!(host.count(), expected);
        assert_eq!(provider.requests().len(), 2);
    }
}

#[tokio::test]
async fn landed_a5_ambiguous_or_malformed_record_containers_retry() {
    for text in [
        format!("{} then {}", record("UNIT-2"), record("UNIT-3")),
        format!("[{}, false]", record("UNIT-2")),
        format!("[{},", record("UNIT-2")),
    ] {
        let provider = Arc::new(MockProvider::new(vec![
            text_response(&text),
            text_response(r#"{"records_landed":1}"#),
        ]));
        let host = retained_host();
        let runner = runner_with(provider.clone(), host.clone(), 4);
        assert_eq!(
            runner.run("continue").await.unwrap(),
            r#"{"records_landed":1}"#
        );
        assert_eq!(provider.requests().len(), 2);
        assert_eq!(host.count(), 1);
    }
}

#[tokio::test]
async fn landed_a7_empty_max_tokens_is_an_authored_output_limit() {
    // Report D7 / matrix #10: output exhausted, not a connection failure.
    for host in [None, Some(retained_host())] {
        let events = vec![
            StreamEvent::MessageDelta {
                stop_reason: Some("max_tokens".into()),
                usage: None,
            },
            StreamEvent::MessageStop,
        ];
        let provider = Arc::new(MockProvider::new(vec![events]));
        let runner = match host {
            Some(host) => runner_with(provider.clone(), host, 3),
            None => make_runner(provider.clone(), 3),
        };
        let answer = runner.run("continue").await;
        if runner.tool_context.audit_landing.is_some() {
            assert_eq!(answer.unwrap(), r#"{"records_landed":1}"#);
        } else {
            let error = answer.unwrap_err();
            assert!(error.to_string().contains("output limit"), "{error}");
        }
        assert_eq!(provider.requests().len(), 1);
    }
}

struct DelayedReply;
#[async_trait::async_trait]
impl LlmProvider for DelayedReply {
    fn name(&self) -> &str {
        "delayed-reply"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!()
    }
    async fn stream(&self, _: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = mpsc::channel(8);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            for event in text_response("unfinished") {
                let _ = tx.send(event).await;
            }
        });
        Ok(rx)
    }
}

#[tokio::test(start_paused = true)]
async fn landed_a1_inference_timeout_completes_from_store() {
    // Report D1 / runtime.rs:188–190: timeout while a reply is in flight.
    let mut runner = make_runner(Arc::new(DelayedReply), 2);
    runner.timeout_secs = 1;
    runner.tool_context.audit_landing = Some(Arc::new(AuditLanding::new(retained_host(), None)));
    assert_eq!(
        runner.run("continue").await.unwrap(),
        r#"{"records_landed":1}"#
    );
}

struct DelayedTool;
#[async_trait::async_trait]
impl Tool for DelayedTool {
    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }
    fn name(&self) -> &str {
        "Delayed"
    }
    fn description(&self) -> &str {
        "Wait for a result"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object"})
    }
    fn permission_level(&self, _: &Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
    async fn execute(&self, _: Value, _: &ToolContext) -> archon_tools::tool::ToolResult {
        tokio::time::sleep(Duration::from_secs(10)).await;
        archon_tools::tool::ToolResult::success("finished")
    }
}

#[tokio::test(start_paused = true)]
async fn landed_a1_tool_timeout_completes_from_store() {
    // Report D1 / runtime.rs:325–327: window cuts an in-flight tool round.
    let provider = Arc::new(MockProvider::new(vec![tool_use_response(
        "slow", "Delayed", "{}",
    )]));
    let mut runner = runner_with(provider, retained_host(), 2);
    runner.timeout_secs = 1;
    let mut registry = crate::dispatch::ToolRegistry::new();
    registry.register(Box::new(DelayedTool));
    runner.tool_definitions = archon_llm::provider::shared_tools(registry.tool_definitions());
    runner.registry = Arc::new(registry);
    assert_eq!(
        runner.run("continue").await.unwrap(),
        r#"{"records_landed":1}"#
    );
}

#[path = "landed_receipt_tests.rs"]
mod landed_receipt_tests;

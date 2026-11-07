// An unframed close is silence for ordinary sessions too.
struct UnframedCloseProvider {
    requests: Mutex<Vec<(tokio::time::Instant, LlmRequest)>>,
    succeed_after: Option<usize>,
}
#[async_trait::async_trait]
impl LlmProvider for UnframedCloseProvider {
    fn name(&self) -> &str {
        "unframed-close"
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
    async fn stream(&self, request: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let count = {
            let mut requests = self.requests.lock().unwrap();
            requests.push((tokio::time::Instant::now(), request));
            requests.len()
        };
        let (tx, rx) = mpsc::channel(4);
        if self.succeed_after.is_some_and(|limit| count > limit) {
            tx.send(StreamEvent::TextDelta {
                index: 0,
                text: "complete".into(),
            })
            .await
            .unwrap();
            tx.send(StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".into()),
                usage: None,
            })
            .await
            .unwrap();
            tx.send(StreamEvent::MessageStop).await.unwrap();
        }
        Ok(rx)
    }
}

#[tokio::test(start_paused = true)]
async fn non_landing_unframed_close_resends_with_exponential_backoff_and_identical_request() {
    let provider = Arc::new(UnframedCloseProvider {
        requests: Mutex::new(vec![]),
        succeed_after: Some(5),
    });
    let runner = runner_over(provider.clone(), 1);
    assert!(runner.tool_context.audit_landing.is_none());
    assert_eq!(runner.run("inspect").await.unwrap(), "complete");
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 6);
    let delays: Vec<_> = requests
        .windows(2)
        .map(|pair| pair[1].0.duration_since(pair[0].0).as_millis())
        .collect();
    assert_eq!(delays, [10, 20, 40, 80, 80]);
    assert!(
        requests
            .iter()
            .all(|(_, request)| request.messages == requests[0].1.messages)
    );
}

#[tokio::test(start_paused = true)]
async fn non_landing_unframed_close_exhausts_the_silence_window() {
    let provider = Arc::new(UnframedCloseProvider {
        requests: Mutex::new(vec![]),
        succeed_after: None,
    });
    let runner = runner_over(provider.clone(), 1);
    let started = tokio::time::Instant::now();
    let error = runner.run("inspect").await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("stream closed without an answer"),
        "{error}"
    );
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    assert!(started.elapsed() < std::time::Duration::from_millis(1100));
    assert!(provider.requests.lock().unwrap().len() > 3);
}

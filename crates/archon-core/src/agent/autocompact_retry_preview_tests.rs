fn assert_retry_preview_lifecycle(
    rx: &mut tokio::sync::mpsc::Receiver<TimestampedEvent>,
    mode: RateLimitFailureMode,
) {
    let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    let lifecycle = events
        .iter()
        .filter_map(|event| match &event.inner {
            AgentEvent::TransientThinkingDelta(text) => Some(text.as_str()),
            AgentEvent::DiscardThinkingPreview => Some("discard"),
            AgentEvent::CommitThinkingPreview => Some("commit"),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = match mode {
        RateLimitFailureMode::PreStream => vec!["accepted preview", "commit"],
        RateLimitFailureMode::MidStream => {
            vec!["rejected preview", "discard", "accepted preview", "commit"]
        }
    };
    assert_eq!(lifecycle, expected);
}

#[tokio::test]
async fn main_pre_stream_rate_limit_compacts_before_one_retry() {
    assert_main_rate_limit_compacts_before_one_retry(RateLimitFailureMode::PreStream).await;
}

#[tokio::test]
async fn main_mid_stream_rate_limit_compacts_before_one_retry() {
    assert_main_rate_limit_compacts_before_one_retry(RateLimitFailureMode::MidStream).await;
}

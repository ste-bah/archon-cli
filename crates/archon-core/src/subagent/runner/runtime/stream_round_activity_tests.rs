// What the runner reports to the host's inactivity clock. Included from
// `stream_round_tests.rs`, which owns the imports and the fixture.

/// Sends `message_start`, then — after `speak_after`, if set — one text delta
/// and a clean stop; otherwise holds the stream open and silent.
struct StartThenMaybeSpeak {
    speak_after: Option<std::time::Duration>,
    started: Mutex<Option<oneshot::Sender<()>>>,
}

#[async_trait::async_trait]
impl LlmProvider for StartThenMaybeSpeak {
    fn name(&self) -> &str {
        "start-then-maybe-speak"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(&self, _: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = mpsc::channel(8);
        let speak_after = self.speak_after;
        tokio::spawn(async move {
            let _ = tx
                .send(StreamEvent::MessageStart {
                    id: "msg".into(),
                    model: "m".into(),
                    usage: Default::default(),
                })
                .await;
            let _ = tx.send(StreamEvent::Ping).await;
            let Some(after) = speak_after else {
                tx.closed().await;
                return;
            };
            tokio::time::sleep(after).await;
            let _ = tx
                .send(StreamEvent::TextDelta {
                    index: 0,
                    text: "done".into(),
                })
                .await;
            let _ = tx.send(StreamEvent::MessageStop).await;
        });
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("this provider only streams")
    }
}

fn activity_runner(provider: StartThenMaybeSpeak, cancel: CancellationToken) -> SubagentRunner {
    SubagentRunner::new(
        Arc::new(provider),
        String::new(),
        Vec::new(),
        Arc::new(crate::dispatch::ToolRegistry::new()),
        ToolContext {
            cancel_parent: Some(cancel),
            ..ToolContext::default()
        },
        "activity-model".into(),
        1,
        86_400,
        Arc::new(crate::agent::AgentConfig::default()),
        Arc::new(test_identity()),
    )
}

fn session_clock(
    clock: &Arc<archon_tools::subagent_activity::ActivityClock>,
) -> archon_tools::subagent_activity::SessionClock {
    archon_tools::subagent_activity::SessionClock {
        agent_id: "activity-session".into(),
        clock: Arc::clone(clock),
    }
}

/// A stalled provider sends `message_start` (and pings) and nothing more. Those
/// must not count, or every resend of the stalled request would reset the bound.
#[tokio::test(start_paused = true)]
async fn message_start_and_pings_are_not_activity() {
    let clock = archon_tools::subagent_activity::ActivityClock::new();
    let (started_tx, started_rx) = oneshot::channel();
    let cancel = CancellationToken::new();
    let runner = activity_runner(
        StartThenMaybeSpeak {
            speak_after: None,
            started: Mutex::new(Some(started_tx)),
        },
        cancel.clone(),
    );
    let run = tokio::spawn(archon_tools::subagent_activity::scope(
        session_clock(&clock),
        async move { runner.run("stall").await },
    ));
    started_rx.await.expect("stream opened");
    let silent_from = clock.silent_since().expect("no tool round");
    tokio::time::sleep(std::time::Duration::from_secs(300)).await;
    assert_eq!(
        clock.silent_since(),
        Some(silent_from),
        "message_start or a ping was counted as activity"
    );
    cancel.cancel();
    let _ = run.await;
}

/// Model output is activity, reported when it arrives.
#[tokio::test(start_paused = true)]
async fn model_output_is_activity() {
    let clock = archon_tools::subagent_activity::ActivityClock::new();
    let cancel = CancellationToken::new();
    let runner = activity_runner(
        StartThenMaybeSpeak {
            speak_after: Some(std::time::Duration::from_secs(120)),
            started: Mutex::new(None),
        },
        cancel,
    );
    let opened = tokio::time::Instant::now();
    let text = archon_tools::subagent_activity::scope(session_clock(&clock), async move {
        runner.run("speak").await
    })
    .await
    .expect("the provider answers");
    assert_eq!(text, "done");
    let last = clock.silent_since().expect("no tool round");
    assert!(
        last >= opened + std::time::Duration::from_secs(120),
        "the text delta did not touch the clock"
    );
}

// Issue 364: a stream that went silent across a machine sleep is resent soon
// after the wake, not one whole idle window of awake time later.

/// Opens streams that never send and never close, as a socket that died while
/// the machine slept looks to its reader; counts every open.
struct SilentAcrossSleepProvider {
    opens: Arc<AtomicU32>,
    held: Mutex<Vec<mpsc::Sender<StreamEvent>>>,
}

#[async_trait::async_trait]
impl LlmProvider for SilentAcrossSleepProvider {
    fn name(&self) -> &str {
        "silent"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(&self, _: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = mpsc::channel(1);
        self.held.lock().unwrap().push(tx);
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("silent provider only streams")
    }
}

async fn wait_for_opens(opens: &AtomicU32, count: u32, limit: std::time::Duration) -> bool {
    tokio::time::timeout(limit, async {
        while opens.load(Ordering::SeqCst) < count {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .is_ok()
}

#[tokio::test]
async fn a_stream_silent_across_a_sleep_is_resent_soon_after_the_wake() {
    let opens = Arc::new(AtomicU32::new(0));
    let config = crate::agent::AgentConfig {
        // An hour of awake time: far longer than this test may take.
        subagent_stream_idle_timeout_secs: 3_600,
        ..Default::default()
    };
    let runner = SubagentRunner::new(
        Arc::new(SilentAcrossSleepProvider {
            opens: Arc::clone(&opens),
            held: Mutex::new(Vec::new()),
        }),
        String::new(),
        Vec::new(),
        Arc::new(crate::dispatch::ToolRegistry::new()),
        ToolContext::default(),
        "silent-model".into(),
        1,
        7_200,
        Arc::new(config),
        Arc::new(test_identity()),
    );
    // Spawned on this test's current-thread runtime, so the wall-clock seam
    // (a thread local) is the one the runner reads.
    let run = tokio::spawn(async move { runner.run("work across a sleep").await });
    assert!(
        wait_for_opens(&opens, 1, std::time::Duration::from_secs(2)).await,
        "the first stream must open"
    );

    // The machine sleeps for seven hours: the wall clock jumps, the monotonic
    // clock does not.
    super::super::stream_idle_window::WALL_JUMP
        .with(|jump| jump.set(std::time::Duration::from_secs(7 * 3_600)));
    let resent = wait_for_opens(&opens, 2, std::time::Duration::from_secs(4)).await;
    super::super::stream_idle_window::WALL_JUMP.with(|jump| jump.set(std::time::Duration::ZERO));
    run.abort();

    assert!(
        resent,
        "a stream silent for the whole idle window by the wall clock must be resent after the \
         wake; opens={}",
        opens.load(Ordering::SeqCst)
    );
}

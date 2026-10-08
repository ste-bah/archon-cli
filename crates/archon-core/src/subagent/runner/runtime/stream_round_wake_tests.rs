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

    // The machine sleeps for seven hours: the wall and boot clocks jump, the
    // monotonic clock does not.
    jump_clocks_as_a_sleep(std::time::Duration::from_secs(7 * 3_600));
    let resent = wait_for_opens(&opens, 2, std::time::Duration::from_secs(4)).await;
    jump_clocks_as_a_sleep(std::time::Duration::ZERO);
    run.abort();

    assert!(
        resent,
        "a stream silent for the whole idle window by the wall clock must be resent after the \
         wake; opens={}",
        opens.load(Ordering::SeqCst)
    );
}

/// Jumps this thread's wall and boot clocks as a sleep of `slept` does.
fn jump_clocks_as_a_sleep(slept: std::time::Duration) {
    super::super::stream_idle_window::WALL_JUMP.with(|jump| jump.set(slept));
    super::super::stream_idle_window::BOOT_JUMP.with(|jump| jump.set(slept));
}

/// What one open of [`WakeNetworkProvider`] does.
#[derive(Clone)]
enum WakeOpen {
    /// A stream that never sends: its socket died in the sleep.
    Silent,
    /// The network is not up yet.
    Unreachable,
    /// The provider refuses the credentials.
    Unauthorized,
    /// The provider answers this text.
    Answers(&'static str),
    /// The provider answers HTTP 500 with this body.
    ServerError(&'static str),
    /// The provider asks for a wait of this many seconds.
    RateLimited(u64),
    /// The stream opens and its first event is an error of this type.
    ErrorEvent(&'static str),
}

/// Opens per a script; the last entry repeats. Counts every open.
struct WakeNetworkProvider {
    script: Vec<WakeOpen>,
    opens: Arc<AtomicU32>,
    held: Mutex<Vec<mpsc::Sender<StreamEvent>>>,
}

#[async_trait::async_trait]
impl LlmProvider for WakeNetworkProvider {
    fn name(&self) -> &str {
        "wake-network"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(&self, _: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let open = self.opens.fetch_add(1, Ordering::SeqCst) as usize;
        let step = self.script[open.min(self.script.len() - 1)].clone();
        let (tx, rx) = mpsc::channel(16);
        match step {
            WakeOpen::Silent => self.held.lock().unwrap().push(tx),
            WakeOpen::Unreachable => {
                return Err(LlmError::Http(
                    "error sending request: network is down".into(),
                ));
            }
            WakeOpen::Unauthorized => {
                return Err(LlmError::Auth("401 invalid x-api-key".into()));
            }
            WakeOpen::ServerError(body) => {
                return Err(LlmError::Server {
                    status: 500,
                    message: body.into(),
                });
            }
            WakeOpen::RateLimited(secs) => {
                return Err(LlmError::RateLimited {
                    retry_after_secs: secs, from_provider: true });
            }
            WakeOpen::ErrorEvent(error_type) => {
                let message = format!("{error_type} from the provider");
                tx.try_send(StreamEvent::Error {
                    error_type: error_type.into(),
                    message,
                })
                .expect("room for the error");
            }
            WakeOpen::Answers(text) => {
                for event in crate::subagent::runner::tests::text_response(text) {
                    tx.try_send(event).expect("room for the reply");
                }
            }
        }
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("the wake provider only streams")
    }
}

fn wake_provider(script: Vec<WakeOpen>, opens: &Arc<AtomicU32>) -> Arc<WakeNetworkProvider> {
    Arc::new(WakeNetworkProvider {
        script,
        opens: Arc::clone(opens),
        held: Mutex::new(Vec::new()),
    })
}

fn wake_runner(script: Vec<WakeOpen>, opens: &Arc<AtomicU32>, idle_secs: u64) -> SubagentRunner {
    runner_over(wake_provider(script, opens), idle_secs)
}

fn runner_over(provider: Arc<dyn LlmProvider>, idle_secs: u64) -> SubagentRunner {
    let config = crate::agent::AgentConfig {
        subagent_stream_idle_timeout_secs: idle_secs,
        ..Default::default()
    };
    SubagentRunner::new(
        provider,
        String::new(),
        Vec::new(),
        Arc::new(crate::dispatch::ToolRegistry::new()),
        ToolContext::default(),
        "wake-model".into(),
        1,
        7_200,
        Arc::new(config),
        Arc::new(test_identity()),
    )
}

/// Fails before the fix: the first resend that could not open ended the
/// round at once, and the network came back seconds later.
#[tokio::test]
async fn resends_while_the_network_comes_back_after_a_wake_complete_the_round() {
    let opens = Arc::new(AtomicU32::new(0));
    let script = vec![
        WakeOpen::Silent,
        WakeOpen::Unreachable,
        WakeOpen::Unreachable,
        WakeOpen::Unreachable,
        WakeOpen::Answers("after the wake"),
    ];
    let runner = wake_runner(script, &opens, 3_600);
    let run = tokio::spawn(async move { runner.run("work across a sleep").await });
    assert!(wait_for_opens(&opens, 1, std::time::Duration::from_secs(2)).await);
    jump_clocks_as_a_sleep(std::time::Duration::from_secs(7 * 3_600));
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), run).await;
    jump_clocks_as_a_sleep(std::time::Duration::ZERO);

    let outcome = outcome.expect("the round must end").expect("no panic");
    assert_eq!(outcome.expect("the round completes"), "after the wake");
    assert_eq!(opens.load(Ordering::SeqCst), 5);
}

/// A network that stays down for a whole no-progress window ends the round
/// with the stall marker, which the workflow host pauses on (never fails).
#[tokio::test]
async fn a_network_down_for_a_whole_window_ends_with_the_resumable_stall_marker() {
    let opens = Arc::new(AtomicU32::new(0));
    let runner = wake_runner(vec![WakeOpen::Silent, WakeOpen::Unreachable], &opens, 1);
    let error = tokio::time::timeout(std::time::Duration::from_secs(20), runner.run("work"))
        .await
        .expect("the round must end")
        .expect_err("no answer for a whole window stops the round")
        .to_string();
    assert!(
        error.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{error}"
    );
    assert!(
        opens.load(Ordering::SeqCst) >= 4,
        "it kept resending inside the window"
    );
}

/// Fails before the fix: a retryable error on the FIRST open of a round (the
/// network not up yet after a wake) ended the round at once.
#[tokio::test]
async fn a_first_open_that_fails_while_the_network_comes_back_completes_the_round() {
    let opens = Arc::new(AtomicU32::new(0));
    let script = vec![
        WakeOpen::Unreachable,
        WakeOpen::Unreachable,
        WakeOpen::Answers("network back"),
    ];
    let runner = wake_runner(script, &opens, 3_600);
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), runner.run("work"))
        .await
        .expect("the round must end");
    assert_eq!(outcome.expect("the round completes"), "network back");
    assert_eq!(opens.load(Ordering::SeqCst), 3);
}

/// Fails before the fix: the round failed with the open error, where a
/// network down for a whole window is a resumable pause.
#[tokio::test]
async fn a_first_open_that_never_succeeds_ends_with_the_resumable_stall_marker() {
    let opens = Arc::new(AtomicU32::new(0));
    let runner = wake_runner(vec![WakeOpen::Unreachable], &opens, 1);
    let started = std::time::Instant::now();
    let error = tokio::time::timeout(std::time::Duration::from_secs(20), runner.run("work"))
        .await
        .expect("the round must end")
        .expect_err("no answer for a whole window stops the round")
        .to_string();
    assert!(
        error.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{error}"
    );
    assert!(
        started.elapsed() >= std::time::Duration::from_secs(1),
        "the stop comes after the whole window, not at once"
    );
    assert!(
        opens.load(Ordering::SeqCst) >= 4,
        "it kept trying inside the window"
    );
}

/// A refused credential is not a network that will come back: it fails at
/// once, with no back-off.
#[tokio::test]
async fn a_first_open_refused_for_auth_fails_fast() {
    let opens = Arc::new(AtomicU32::new(0));
    let runner = wake_runner(vec![WakeOpen::Unauthorized], &opens, 3_600);
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), runner.run("work"))
        .await
        .expect("an auth failure must not wait out a window")
        .expect_err("an auth failure fails the round")
        .to_string();
    assert!(
        !error.contains(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{error}"
    );
    assert!(error.contains("authentication error"), "{error}");
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

/// Fails before the fix: the awake silence from before the sleep counted, so
/// the round stopped a few quick failures after the wake while the network
/// was still coming back.
#[tokio::test]
async fn a_stream_quiet_before_a_sleep_has_a_whole_window_after_the_wake() {
    let opens = Arc::new(AtomicU32::new(0));
    let mut script = vec![WakeOpen::Silent];
    script.extend(std::iter::repeat_n(WakeOpen::Unreachable, 8));
    script.push(WakeOpen::Answers("after the wake"));
    let runner = wake_runner(script, &opens, 2);
    let run = tokio::spawn(async move { runner.run("work across a sleep").await });
    assert!(wait_for_opens(&opens, 1, std::time::Duration::from_secs(2)).await);
    // Most of the window passes awake and silent, then the machine sleeps.
    tokio::time::sleep(std::time::Duration::from_millis(1_600)).await;
    jump_clocks_as_a_sleep(std::time::Duration::from_secs(7 * 3_600));
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), run).await;
    jump_clocks_as_a_sleep(std::time::Duration::ZERO);

    let outcome = outcome.expect("the round must end").expect("no panic");
    assert_eq!(outcome.expect("the round completes"), "after the wake");
    assert_eq!(opens.load(Ordering::SeqCst), 10);
}

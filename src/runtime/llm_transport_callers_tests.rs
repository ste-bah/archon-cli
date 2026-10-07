//! #356: the long read backstop serves workflow (judge/critic) clients only.
//! A command surface keeps its own idle bound: a dead stream ends there, not
//! after the judge window.
use super::*;
use std::time::Duration;

/// A real call over the real configured transport, with no activity window of
/// its own: only the HTTP read backstop can end a silent stream.
async fn silent_stream_ends_at(callers: TransportCallers, local: bool, ends: u64) {
    let (client, mut transport) = client_with(None, true, local, callers).await;
    // Counts real stream events: the opening keep-alive must be consumed (and
    // the next read armed) before time is paused, or the bound starts late.
    let events = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = events.clone();
    let progress = archon_shell::progress::Progress::with_clock(
        false,
        Arc::new(move || {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::Instant::now()
        }),
    );
    let call = tokio::spawn(async move {
        client
            .send_message_with_progress(
                vec![serde_json::json!({"role": "user", "content": "hello"})],
                Vec::new(),
                Vec::new(),
                "fixture-model",
                0.0,
                progress,
            )
            .await
    });
    transport.ready().await;
    let started = std::time::Instant::now();
    // `with_clock` reads the clock once at construction, then once per record.
    while events.load(std::sync::atomic::Ordering::SeqCst) < 2 {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "keep-alive never arrived"
        );
        tokio::task::yield_now().await;
    }
    settle().await;
    tokio::time::pause();
    // Any earlier class bound must not apply: alive up to just before `ends`.
    let mut now = 0;
    for at in [ends / 2, ends - 60] {
        tokio::time::advance(Duration::from_secs(at - now)).await;
        now = at;
        settle().await;
        assert!(
            !call.is_finished(),
            "{callers:?} stream cut at {at}s, before {ends}s"
        );
    }
    // Timer and wake-up granularity: the bound holds within a minute either side.
    tokio::time::advance(Duration::from_secs(120)).await;
    // The expiry crosses reqwest, the provider task, the provider observer and
    // two adapters. The observer persists the stream error on a blocking
    // thread before it forwards it (`provider_observer_stream.rs`), and that
    // write takes real time, so a fixed count of scheduler turns is a race
    // with the disk. Wait for the outcome itself with the virtual clock held
    // at the bound plus a minute: yielding keeps the paused clock from
    // auto-advancing, and the clock is checked unmoved below, so only the
    // configured bound can have ended the stream. The real-time limit only
    // turns a hang into a failure.
    let at_bound = tokio::time::Instant::now();
    let waiting = std::time::Instant::now();
    while !call.is_finished() {
        assert!(
            waiting.elapsed() < Duration::from_secs(30),
            "{callers:?} dead stream outlived its {ends}s bound"
        );
        settle().await;
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        tokio::time::Instant::now(),
        at_bound,
        "{callers:?} stream ended only after the clock moved past {ends}s + 60s"
    );
    let error = call
        .await
        .unwrap()
        .expect_err("a silent stream yields no reply");
    assert!(
        matches!(error, archon_workflow::WorkflowError::ControlPaused(_)),
        "a transport idle stop stays resumable: {error:#}"
    );
    tokio::time::resume();
}

#[test]
fn issue356_backstop_by_caller_class() {
    let mut config = ArchonConfig::default();
    assert_eq!(
        provider_read_backstop(&config, TransportCallers::Command),
        1_800
    );
    assert_eq!(
        provider_read_backstop(&config, TransportCallers::Workflow),
        7_800
    );
    // A raised subagent guard still wins for both classes.
    config.subagent.stream_idle_timeout_secs = 9_000;
    assert_eq!(
        provider_read_backstop(&config, TransportCallers::Command),
        9_600
    );
    assert_eq!(
        provider_read_backstop(&config, TransportCallers::Workflow),
        9_600
    );
}

#[tokio::test]
async fn issue356_command_anthropic_dead_stream_ends_at_its_idle_bound() {
    silent_stream_ends_at(TransportCallers::Command, false, 1_800).await;
}
#[tokio::test]
async fn issue356_workflow_anthropic_dead_stream_outlasts_judge_window() {
    silent_stream_ends_at(TransportCallers::Workflow, false, 7_800).await;
}
#[tokio::test]
async fn issue356_command_local_dead_stream_ends_at_configured_timeout() {
    // The fixture configures `[llm.local] timeout_secs = 300`.
    silent_stream_ends_at(TransportCallers::Command, true, 300).await;
}
#[tokio::test]
async fn issue356_workflow_local_dead_stream_outlasts_judge_window() {
    silent_stream_ends_at(TransportCallers::Workflow, true, 7_800).await;
}

use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;

fn timestamped(inner: AgentEvent) -> TimestampedEvent {
    TimestampedEvent {
        sent_at: Instant::now(),
        inner,
    }
}

fn rendered_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

#[tokio::test]
async fn large_tool_output_reaches_tui_as_bounded_ordered_frames() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let session_store = Arc::new(
        archon_session::storage::SessionStore::open(&temp.path().join("sessions.db"))
            .expect("open session store"),
    );
    let (agent_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let (tui_tx, mut tui_rx) =
        archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    let cost_config = archon_core::config::CostConfig::default();
    spawn_agent_event_forwarder(AgentEventForwarderConfig {
        event_rx,
        metrics: Arc::new(archon_tui::observability::ChannelMetrics::default()),
        tui_tx,
        session_stats: Arc::new(tokio::sync::Mutex::new(SessionStats::default())),
        cost_alert_state: CostAlertState::new(&cost_config),
        cost_config,
        active_session: crate::session::active_session::ActiveSessionId::new("large-output-test"),
        session_store,
        permission_mode: Arc::new(tokio::sync::Mutex::new("auto".into())),
        permission_events_db: None,
        agent_ledger_db: None,
        ledger_context: crate::runtime::agent_ledger_events::AgentLedgerContext::new(
            "main",
            "large-output-test",
            "test-model",
            "test-provider",
        ),
        selected_model: "test-model".into(),
    });
    let output = format!("{}世界", "0123456789abcdef\n".repeat(5_000));
    agent_tx
        .send(timestamped(AgentEvent::ToolCallComplete {
            name: "Bash".into(),
            id: "tool-large".into(),
            result: archon_tools::tool::ToolResult::success(output.clone()),
            transcript_summary: None,
        }))
        .await
        .unwrap();

    let mut reconstructed = String::new();
    loop {
        match tui_rx.recv().await.expect("framed tool output") {
            TuiEvent::ToolOutputChunk { id, chunk } => {
                assert_eq!(id, "tool-large");
                assert!(chunk.len() <= archon_tui::event_channel::MAX_COALESCED_CONTENT_BYTES);
                reconstructed.push_str(&chunk);
            }
            TuiEvent::ToolComplete { id, output, .. } => {
                assert_eq!(id, "tool-large");
                assert!(output.is_empty());
                break;
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }
    assert_eq!(reconstructed, output);
}

#[tokio::test]
async fn full_tui_queue_backpressures_agent_source_without_reordering() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let session_store = Arc::new(
        archon_session::storage::SessionStore::open(&temp.path().join("sessions.db"))
            .expect("open session store"),
    );
    let (agent_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let (tui_tx, mut tui_rx) =
        archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    let cost_config = archon_core::config::CostConfig::default();
    spawn_agent_event_forwarder(AgentEventForwarderConfig {
        event_rx,
        metrics: Arc::new(archon_tui::observability::ChannelMetrics::default()),
        tui_tx,
        session_stats: Arc::new(tokio::sync::Mutex::new(SessionStats::default())),
        cost_alert_state: CostAlertState::new(&cost_config),
        cost_config,
        active_session: crate::session::active_session::ActiveSessionId::new("pressure-test"),
        session_store,
        permission_mode: Arc::new(tokio::sync::Mutex::new("auto".into())),
        permission_events_db: None,
        agent_ledger_db: None,
        ledger_context: crate::runtime::agent_ledger_events::AgentLedgerContext::new(
            "main",
            "pressure-test",
            "test-model",
            "test-provider",
        ),
        selected_model: "test-model".into(),
    });

    agent_tx
        .send(timestamped(AgentEvent::TextDelta("one".into())))
        .await
        .unwrap();
    while tui_rx.is_empty() {
        tokio::task::yield_now().await;
    }
    let remaining = tokio::spawn(async move {
        agent_tx
            .send(timestamped(AgentEvent::TransientThinkingDelta(
                "preview".into(),
            )))
            .await?;
        agent_tx
            .send(timestamped(AgentEvent::ThinkingDelta("two".into())))
            .await?;
        agent_tx
            .send(timestamped(AgentEvent::TextDelta("three".into())))
            .await?;
        agent_tx
            .send(timestamped(AgentEvent::SessionComplete))
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), async {
            while !remaining.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err(),
        "agent source did not inherit TUI backpressure"
    );

    assert!(matches!(tui_rx.recv().await, Some(TuiEvent::TextDelta(text)) if text == "one"));
    assert!(
        matches!(tui_rx.recv().await, Some(TuiEvent::TransientThinkingDelta(text)) if text == "preview")
    );
    assert!(matches!(tui_rx.recv().await, Some(TuiEvent::ThinkingDelta(text)) if text == "two"));
    remaining
        .await
        .expect("remaining sender task")
        .expect("remaining events should resume after drain");
    assert!(matches!(tui_rx.recv().await, Some(TuiEvent::TextDelta(text)) if text == "three"));
    assert!(matches!(tui_rx.recv().await, Some(TuiEvent::Done)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_text_reconstructs_response_and_rendered_transcript() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let session_store = Arc::new(
        archon_session::storage::SessionStore::open(&temp.path().join("sessions.db"))
            .expect("open session store"),
    );
    let (agent_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let (tui_tx, event_rx_tui) =
        archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    let (input_tx, _input_rx) = tokio::sync::mpsc::channel(1);
    let cost_config = archon_core::config::CostConfig::default();
    let last_response = spawn_agent_event_forwarder(AgentEventForwarderConfig {
        event_rx,
        metrics: Arc::new(archon_tui::observability::ChannelMetrics::default()),
        tui_tx,
        session_stats: Arc::new(tokio::sync::Mutex::new(SessionStats::default())),
        cost_alert_state: CostAlertState::new(&cost_config),
        cost_config,
        active_session: crate::session::active_session::ActiveSessionId::new("test-session"),
        session_store,
        permission_mode: Arc::new(tokio::sync::Mutex::new("auto".into())),
        permission_events_db: None,
        agent_ledger_db: None,
        ledger_context: crate::runtime::agent_ledger_events::AgentLedgerContext::new(
            "main",
            "test-session",
            "test-model",
            "test-provider",
        ),
        selected_model: "test-model".into(),
    });
    let expected = "hello 世界\nfinal";
    for chunk in ["hello ", "世界", "\nfinal"] {
        agent_tx
            .send(timestamped(AgentEvent::TextDelta(chunk.into())))
            .await
            .expect("send text chunk");
    }

    let response_for_shutdown = Arc::clone(&last_response);
    let shutdown = tokio::spawn(async move {
        loop {
            if response_for_shutdown.lock().await.as_str() == expected {
                break;
            }
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        agent_tx
            .send(timestamped(AgentEvent::SessionComplete))
            .await
            .expect("send session completion");
    });
    let config = archon_tui::app::AppConfig {
        event_rx: event_rx_tui,
        input_tx,
        model: "test-model".into(),
        splash: None,
        btw_tx: None,
        permission_tx: None,
        ask_user_tx: None,
        context_window: 0,
        context_source: None,
        context_threshold: 0.8,
        command_catalog: Vec::new(),
    };
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("create terminal");

    tokio::time::timeout(
        Duration::from_secs(5),
        archon_tui::app::run_with_backend_without_terminal_events(config, &mut terminal),
    )
    .await
    .expect("headless TUI timed out")
    .expect("headless TUI failed");
    shutdown.await.expect("shutdown task failed");

    assert_eq!(*last_response.lock().await, expected);
    let rendered = rendered_text(&terminal);
    let mut lines = rendered.lines();
    let first_line = lines.next().expect("render first line");
    assert!(first_line.starts_with("hello "));
    let world = first_line.find('世').expect("render first wide character");
    let boundary = first_line.find('界').expect("render second wide character");
    assert!(world < boundary, "wide characters must preserve order");
    assert_eq!(lines.next().map(str::trim_end), Some("final"));
}

/// Issue #37: `/context` has to be able to report the size of the last request
/// body, and the forwarder is the only place that sees it.
///
/// The preflight `ContextPressureUpdated` is emitted before the request is
/// sent, so this is the one number that survives a rate-limited turn — a turn
/// that never reaches `TurnComplete` and therefore never bills an input token.
/// Deleting the `session_stats` write in the forwarder leaves the counter at
/// zero and fails this test, while the TUI event keeps flowing unchanged.
#[tokio::test]
async fn context_pressure_event_banks_last_request_body_tokens_for_slash_context() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let session_store = Arc::new(
        archon_session::storage::SessionStore::open(&temp.path().join("sessions.db"))
            .expect("open session store"),
    );
    let (agent_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let (tui_tx, mut tui_rx) =
        archon_tui::event_channel::bounded_tui_event_channel_with_capacity(4);
    let cost_config = archon_core::config::CostConfig::default();
    let session_stats = Arc::new(tokio::sync::Mutex::new(SessionStats::default()));
    spawn_agent_event_forwarder(AgentEventForwarderConfig {
        event_rx,
        metrics: Arc::new(archon_tui::observability::ChannelMetrics::default()),
        tui_tx,
        session_stats: Arc::clone(&session_stats),
        cost_alert_state: CostAlertState::new(&cost_config),
        cost_config,
        active_session: crate::session::active_session::ActiveSessionId::new("pressure-stats"),
        session_store,
        permission_mode: Arc::new(tokio::sync::Mutex::new("auto".into())),
        permission_events_db: None,
        agent_ledger_db: None,
        ledger_context: crate::runtime::agent_ledger_events::AgentLedgerContext::new(
            "main",
            "pressure-stats",
            "test-model",
            "test-provider",
        ),
        selected_model: "test-model".into(),
    });

    assert_eq!(
        session_stats.lock().await.last_request_body_tokens,
        0,
        "a session with no request sent must report zero"
    );

    for tokens in [470_000u64, 180_000u64] {
        agent_tx
            .send(timestamped(AgentEvent::ContextPressureUpdated {
                tokens_used: tokens,
                context_window: 1_000_000,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
                context_name: Some("main".into()),
                resolution_source: Some("bundled-catalog".into()),
            }))
            .await
            .expect("send context pressure event");

        // The forwarded TUI event is the completion signal for this iteration:
        // it is sent after the stats write, so observing it means the write has
        // already landed.
        match tui_rx.recv().await.expect("forwarded pressure event") {
            TuiEvent::ContextPressureUpdated { tokens_used, .. } => {
                assert_eq!(tokens_used, tokens, "the TUI event must pass through");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        assert_eq!(
            session_stats.lock().await.last_request_body_tokens,
            tokens,
            "the latest preflight size must overwrite the previous one"
        );
    }
}

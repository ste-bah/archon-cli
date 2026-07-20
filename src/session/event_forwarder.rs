use std::sync::Arc;

use archon_cli_workspace::event_coalescer::{EventCoalescer, RENDER_EVENT_BUDGET};
use archon_core::agent::{AgentEvent, SessionStats, TimestampedEvent};
use archon_core::cost_alerts::{CostAlertAction, CostAlertState};
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::observability;

pub(super) struct AgentEventForwarderConfig {
    pub event_rx: tokio::sync::mpsc::UnboundedReceiver<TimestampedEvent>,
    pub metrics: Arc<archon_tui::observability::ChannelMetrics>,
    pub tui_tx: TuiEventSender,
    pub session_stats: Arc<tokio::sync::Mutex<SessionStats>>,
    pub cost_alert_state: CostAlertState,
    pub cost_config: archon_core::config::CostConfig,
    pub session_id: String,
    pub session_store: Arc<archon_session::storage::SessionStore>,
    pub permission_mode: Arc<tokio::sync::Mutex<String>>,
    pub permission_events_db: Option<Arc<cozo::DbInstance>>,
    pub agent_ledger_db: Option<Arc<cozo::DbInstance>>,
    pub ledger_context: crate::runtime::agent_ledger_events::AgentLedgerContext,
    pub selected_model: String,
}

pub(super) fn spawn_agent_event_forwarder(
    config: AgentEventForwarderConfig,
) -> Arc<tokio::sync::Mutex<String>> {
    let last_assistant_response = Arc::new(tokio::sync::Mutex::new(String::new()));
    let last_response_for_fwd = Arc::clone(&last_assistant_response);
    let AgentEventForwarderConfig {
        mut event_rx,
        metrics,
        tui_tx,
        session_stats,
        mut cost_alert_state,
        cost_config,
        session_id,
        session_store,
        permission_mode,
        permission_events_db,
        agent_ledger_db,
        ledger_context,
        selected_model,
    } = config;
    observability::spawn_named("agent-event-forwarder", async move {
        let mut coalescer = EventCoalescer::with_defaults();
        loop {
            let timestamped = match event_rx.recv().await {
                Some(ts) => ts,
                None => break,
            };
            let elapsed_ms = (timestamped.sent_at.elapsed().as_millis() as u64).max(1);
            metrics.record_latency_ms(elapsed_ms);
            coalescer.push(timestamped.inner);

            let mut drained = 1usize;
            while drained < RENDER_EVENT_BUDGET {
                match event_rx.try_recv() {
                    Ok(ts) => {
                        let elapsed = (ts.sent_at.elapsed().as_millis() as u64).max(1);
                        metrics.record_latency_ms(elapsed);
                        coalescer.push(ts.inner);
                        drained += 1;
                    }
                    Err(_) => break,
                }
            }
            metrics.record_drained(drained as u64);
            let _ = metrics.warn_if_backlog_over(10_000);

            while let Some(event) = coalescer.pop() {
                let tui_event = match event {
                    AgentEvent::TextDelta(text) => {
                        last_response_for_fwd.lock().await.push_str(&text);
                        TuiEvent::TextDelta(text)
                    }
                    AgentEvent::ThinkingDelta(text) => TuiEvent::ThinkingDelta(text),
                    AgentEvent::ToolCallStarted { name, id } => TuiEvent::ToolStart { name, id },
                    AgentEvent::ToolCallComplete {
                        name,
                        id,
                        result,
                        transcript_summary,
                    } => TuiEvent::ToolComplete {
                        name,
                        id,
                        success: !result.is_error,
                        output: result.content,
                        transcript_summary,
                    },
                    AgentEvent::ContextPressureUpdated {
                        tokens_used,
                        context_window,
                        cache_creation_tokens,
                        cache_read_tokens,
                        context_name,
                        resolution_source,
                    } => TuiEvent::ContextPressureUpdated {
                        tokens_used,
                        context_window,
                        cache_creation_tokens,
                        cache_read_tokens,
                        context_name,
                        resolution_source,
                    },
                    AgentEvent::TurnComplete {
                        input_tokens,
                        output_tokens,
                        cache_creation_tokens,
                        cache_read_tokens,
                    } => {
                        handle_turn_complete(
                            input_tokens,
                            output_tokens,
                            cache_creation_tokens,
                            cache_read_tokens,
                            &session_stats,
                            &cost_config,
                            &mut cost_alert_state,
                            &tui_tx,
                            &session_store,
                            &session_id,
                            &permission_mode,
                            &agent_ledger_db,
                            &ledger_context,
                            &selected_model,
                        )
                        .await
                    }
                    AgentEvent::Error(msg) => {
                        let mode = permission_mode.lock().await.clone();
                        crate::runtime::agent_ledger_events::record_agent_runtime_error(
                            agent_ledger_db.as_ref(),
                            &ledger_context,
                            &mode,
                        );
                        TuiEvent::Error(msg)
                    }
                    AgentEvent::SessionComplete => TuiEvent::Done,
                    AgentEvent::PermissionRequired { tool, description } => {
                        record_permission(
                            permission_events_db.as_ref(),
                            &session_id,
                            &ledger_context,
                            &permission_mode,
                            &tool,
                            "requested",
                            None,
                        )
                        .await;
                        TuiEvent::PermissionPrompt { tool, description }
                    }
                    AgentEvent::AskUser { question } => TuiEvent::AskUserPrompt { question },
                    AgentEvent::PermissionGranted { tool } => {
                        record_permission(
                            permission_events_db.as_ref(),
                            &session_id,
                            &ledger_context,
                            &permission_mode,
                            &tool,
                            "granted",
                            None,
                        )
                        .await;
                        continue;
                    }
                    AgentEvent::PermissionDenied { tool, reason } => {
                        record_permission(
                            permission_events_db.as_ref(),
                            &session_id,
                            &ledger_context,
                            &permission_mode,
                            &tool,
                            "denied",
                            reason.as_deref(),
                        )
                        .await;
                        continue;
                    }
                    _ => continue,
                };
                if tui_tx.send(tui_event).is_err() {
                    return;
                }
            }
        }
    });
    last_assistant_response
}

#[allow(clippy::too_many_arguments)]
async fn handle_turn_complete(
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    session_stats: &Arc<tokio::sync::Mutex<SessionStats>>,
    cost_config: &archon_core::config::CostConfig,
    cost_alert_state: &mut CostAlertState,
    tui_tx: &TuiEventSender,
    session_store: &archon_session::storage::SessionStore,
    session_id: &str,
    permission_mode: &Arc<tokio::sync::Mutex<String>>,
    agent_ledger_db: &Option<Arc<cozo::DbInstance>>,
    ledger_context: &crate::runtime::agent_ledger_events::AgentLedgerContext,
    selected_model: &str,
) -> TuiEvent {
    let estimated_cost = {
        let stats = session_stats.lock().await;
        archon_core::cost::estimate_session_cost_usd(
            selected_model,
            stats.input_tokens,
            stats.output_tokens,
            stats.cache_stats.cache_creation_tokens,
            stats.cache_stats.cache_read_tokens,
        )
    };

    match cost_alert_state.check_cost(estimated_cost, cost_config) {
        CostAlertAction::Warn(msg) => {
            let _ = tui_tx.send(TuiEvent::Error(format!("COST WARNING: {msg}")));
        }
        CostAlertAction::HardLimitPause(msg) => {
            let _ = tui_tx.send(TuiEvent::Error(format!("COST LIMIT: {msg}")));
        }
        CostAlertAction::None => {}
    }

    {
        let stats = session_stats.lock().await;
        let _ = session_store.update_usage(
            session_id,
            stats.input_tokens + stats.output_tokens,
            estimated_cost,
        );
    }

    let mode = permission_mode.lock().await.clone();
    crate::runtime::agent_ledger_events::record_agent_turn_completed(
        agent_ledger_db.as_ref(),
        ledger_context,
        &mode,
        input_tokens,
        output_tokens,
    );

    TuiEvent::TurnComplete {
        input_tokens,
        output_tokens,
        cache_creation_tokens,
        cache_read_tokens,
    }
}

async fn record_permission(
    db: Option<&Arc<cozo::DbInstance>>,
    session_id: &str,
    ledger_context: &crate::runtime::agent_ledger_events::AgentLedgerContext,
    permission_mode: &Arc<tokio::sync::Mutex<String>>,
    tool: &str,
    decision: &str,
    reason: Option<&str>,
) {
    let mode = permission_mode.lock().await.clone();
    crate::runtime::permission_events::record_permission_event(
        db,
        session_id,
        Some(&ledger_context.agent_type),
        &mode,
        tool,
        decision,
        reason,
    );
}

#[cfg(test)]
mod tests {
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn agent_text_reconstructs_response_and_rendered_transcript() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let session_store = Arc::new(
            archon_session::storage::SessionStore::open(&temp.path().join("sessions.db"))
                .expect("open session store"),
        );
        let (agent_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
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
            session_id: "test-session".into(),
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
}

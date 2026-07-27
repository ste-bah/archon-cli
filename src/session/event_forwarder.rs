use std::sync::Arc;

use archon_core::agent::{AgentEvent, SessionStats, TimestampedEvent};
use archon_core::cost_alerts::{CostAlertAction, CostAlertState};
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::observability;

pub(super) struct AgentEventForwarderConfig {
    pub event_rx: tokio::sync::mpsc::Receiver<TimestampedEvent>,
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
        while let Some(timestamped) = event_rx.recv().await {
            let elapsed_ms = (timestamped.sent_at.elapsed().as_millis() as u64).max(1);
            metrics.record_latency_ms(elapsed_ms);
            metrics.record_drained(1);
            let _ = metrics.warn_if_backlog_over(10_000);

            let tui_event = match timestamped.inner {
                AgentEvent::TextDelta(text) => {
                    last_response_for_fwd.lock().await.push_str(&text);
                    TuiEvent::TextDelta(text)
                }
                AgentEvent::ThinkingDelta(text) => TuiEvent::ThinkingDelta(text),
                AgentEvent::TransientThinkingDelta(text) => TuiEvent::TransientThinkingDelta(text),
                AgentEvent::CommitThinkingPreview => TuiEvent::CommitThinkingPreview,
                AgentEvent::DiscardThinkingPreview => TuiEvent::DiscardThinkingPreview,
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
                    let events = handle_turn_complete(
                        input_tokens,
                        output_tokens,
                        cache_creation_tokens,
                        cache_read_tokens,
                        &session_stats,
                        &cost_config,
                        &mut cost_alert_state,
                        &session_store,
                        &session_id,
                        &permission_mode,
                        &agent_ledger_db,
                        &ledger_context,
                        &selected_model,
                    )
                    .await;
                    for event in events {
                        if tui_tx.send_async(event).await.is_err() {
                            return;
                        }
                    }
                    continue;
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
            if tui_tx.send_async(tui_event).await.is_err() {
                return;
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
    session_store: &archon_session::storage::SessionStore,
    session_id: &str,
    permission_mode: &Arc<tokio::sync::Mutex<String>>,
    agent_ledger_db: &Option<Arc<cozo::DbInstance>>,
    ledger_context: &crate::runtime::agent_ledger_events::AgentLedgerContext,
    selected_model: &str,
) -> Vec<TuiEvent> {
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

    let mut events = Vec::with_capacity(2);
    match cost_alert_state.check_cost(estimated_cost, cost_config) {
        CostAlertAction::Warn(msg) => events.push(TuiEvent::Error(format!("COST WARNING: {msg}"))),
        CostAlertAction::HardLimitPause(msg) => {
            events.push(TuiEvent::Error(format!("COST LIMIT: {msg}")))
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

    events.push(TuiEvent::TurnComplete {
        input_tokens,
        output_tokens,
        cache_creation_tokens,
        cache_read_tokens,
    });
    events
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
#[path = "event_forwarder_tests.rs"]
mod tests;

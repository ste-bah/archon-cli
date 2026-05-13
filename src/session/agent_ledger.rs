use std::sync::Arc;

use archon_core::agent::{AgentEvent, TimestampedEvent};

pub(super) fn context(
    session_id: &str,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
    model: impl Into<String>,
    provider: impl Into<String>,
) -> crate::runtime::agent_ledger_events::AgentLedgerContext {
    crate::runtime::agent_ledger_events::AgentLedgerContext::new(
        agent_def
            .map(|def| def.agent_type.clone())
            .unwrap_or_else(|| "main".into()),
        session_id.to_string(),
        model,
        provider,
    )
    .with_version(agent_def.map(|def| def.meta.version.clone()))
}

pub(super) async fn record_event(
    db: Option<&Arc<cozo::DbInstance>>,
    context: &crate::runtime::agent_ledger_events::AgentLedgerContext,
    permission_mode: &Arc<tokio::sync::Mutex<String>>,
    event: &AgentEvent,
) {
    let mode = permission_mode.lock().await.clone();
    match event {
        AgentEvent::TurnComplete {
            input_tokens,
            output_tokens,
            ..
        } => crate::runtime::agent_ledger_events::record_agent_turn_completed(
            db,
            context,
            &mode,
            *input_tokens,
            *output_tokens,
        ),
        AgentEvent::Error(_) => {
            crate::runtime::agent_ledger_events::record_agent_runtime_error(db, context, &mode)
        }
        _ => {}
    }
}

pub(super) fn spawn_print_forwarder(
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<TimestampedEvent>,
    db: Option<Arc<cozo::DbInstance>>,
    context: crate::runtime::agent_ledger_events::AgentLedgerContext,
    permission_mode: Arc<tokio::sync::Mutex<String>>,
) -> tokio::sync::mpsc::UnboundedReceiver<TimestampedEvent> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    archon_tui::observability::spawn_named("print-agent-ledger-forwarder", async move {
        while let Some(timestamped) = event_rx.recv().await {
            record_event(db.as_ref(), &context, &permission_mode, &timestamped.inner).await;
            if tx.send(timestamped).is_err() {
                break;
            }
        }
    });
    rx
}

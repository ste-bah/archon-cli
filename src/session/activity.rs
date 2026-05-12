use std::sync::Arc;

use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;

#[derive(Debug, Clone)]
struct SessionActivitySink {
    jsonl: archon_observability::JsonlActivitySink,
    tui_tx: Option<TuiEventSender>,
}

impl archon_observability::AgentActivitySink for SessionActivitySink {
    fn emit(&self, event: archon_observability::AgentActivityEvent) {
        archon_observability::AgentActivitySink::emit(&self.jsonl, event.clone());
        if let Some(tx) = &self.tui_tx {
            let _ = tx.send(TuiEvent::AgentActivity(event.into()));
        }
    }
}

pub(super) fn session_activity_sink(
    session_id: &str,
) -> Option<Arc<dyn archon_observability::AgentActivitySink>> {
    session_activity_sink_inner(session_id, None)
}

pub(super) fn session_activity_sink_with_tui(
    session_id: &str,
    tui_tx: TuiEventSender,
) -> Option<Arc<dyn archon_observability::AgentActivitySink>> {
    session_activity_sink_inner(session_id, Some(tui_tx))
}

fn session_activity_sink_inner(
    session_id: &str,
    tui_tx: Option<TuiEventSender>,
) -> Option<Arc<dyn archon_observability::AgentActivitySink>> {
    let base_dir = dirs::home_dir()?.join(".archon/sessions");
    let path = archon_observability::activity_jsonl_path(base_dir, session_id);
    Some(Arc::new(SessionActivitySink {
        jsonl: archon_observability::JsonlActivitySink::new(path),
        tui_tx,
    }))
}

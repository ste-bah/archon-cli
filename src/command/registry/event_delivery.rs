use archon_tui::app::TuiEvent;

use super::CommandContext;

const MAX_PENDING_TUI_EVENTS: usize = 64;
const PENDING_TUI_REJECTION: &str =
    "Command output exceeded the bounded command event buffer and was rejected.";

impl CommandContext {
    /// Deliver immediately when capacity exists. After saturation, retain a
    /// bounded FIFO prefix plus one rejection marker for async flushing.
    pub(crate) fn emit(&self, event: TuiEvent) {
        let mut pending = self
            .pending_tui_events
            .lock()
            .expect("pending TUI event lock");
        if !pending.is_empty() {
            push_pending_event(&mut pending, event);
            return;
        }
        if let Err(tokio::sync::mpsc::error::SendError(event)) = self.tui_tx.send(event) {
            push_pending_event(&mut pending, event);
        }
    }

    /// Deliver queued handler events with bounded-channel backpressure.
    pub(crate) async fn flush_events(
        &self,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<TuiEvent>> {
        let events = {
            let mut pending = self
                .pending_tui_events
                .lock()
                .expect("pending TUI event lock");
            std::mem::take(&mut *pending)
        };
        for event in events {
            self.tui_tx.send_async(event).await?;
        }
        Ok(())
    }
}

fn push_pending_event(pending: &mut Vec<TuiEvent>, event: TuiEvent) {
    let oversized = archon_tui::event_channel::retained_event_bytes(&event)
        > archon_tui::event_channel::MAX_COALESCED_CONTENT_BYTES;
    if oversized || pending.len() >= MAX_PENDING_TUI_EVENTS - 1 {
        if !pending.iter().any(
            |event| matches!(event, TuiEvent::Error(message) if message == PENDING_TUI_REJECTION),
        ) && pending.len() < MAX_PENDING_TUI_EVENTS
        {
            pending.push(TuiEvent::Error(PENDING_TUI_REJECTION.to_string()));
        }
        return;
    }
    pending.push(event);
}

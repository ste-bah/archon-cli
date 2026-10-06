//! Bounded activity state used to reconcile the rail after queue overflow.
use crate::events::{AgentActivityStatus, AgentActivityUpdate, TuiEvent};

#[derive(Debug, Default)]
pub(super) struct ActivityState {
    pub rows: Vec<AgentActivityUpdate>,
    pub dirty: bool,
}
impl ActivityState {
    pub fn terminal_without_detail(&mut self, update: &AgentActivityUpdate) {
        if crate::agent_activity::is_terminal_non_parent(update.role, update.status) {
            self.rows.retain(|row| row.id != update.id);
        } else if let Some(row) = self.rows.iter_mut().find(|row| row.id == update.id) {
            row.status = update.status;
            row.detail = None;
            row.current_tool = None;
        }
    }
    pub fn observe(&mut self, update: &AgentActivityUpdate) {
        let terminal = crate::agent_activity::is_terminal_non_parent(update.role, update.status);
        if terminal {
            self.rows.retain(|row| row.id != update.id);
        } else if let Some(row) = self.rows.iter_mut().find(|row| row.id == update.id) {
            *row = update.clone();
        } else {
            self.rows.push(update.clone());
            // The same bound as the visible rail: reconciliation never needs
            // an unbounded backlog of terminal events or asynchronous tasks.
            if self.rows.len() > crate::agent_activity::MAX_ACTIVITY_ROWS {
                self.rows.remove(0);
            }
        }
    }
}

/// Completion updates need reconciliation even for a retained parent row.
pub(super) fn terminal(event: &TuiEvent) -> bool {
    matches!(event, TuiEvent::AgentActivity(update) if matches!(update.status,
        AgentActivityStatus::Complete | AgentActivityStatus::Failed | AgentActivityStatus::Cancelled))
}

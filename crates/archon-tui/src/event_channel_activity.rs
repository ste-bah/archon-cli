//! Bounded activity state used to reconcile the rail after queue overflow.
use crate::events::AgentActivityUpdate;

#[derive(Debug, Default)]
pub(super) struct ActivityState {
    pub rows: Vec<AgentActivityUpdate>,
    pub dirty: bool,
}
impl ActivityState {
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

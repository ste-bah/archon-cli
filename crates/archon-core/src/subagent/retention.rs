//! Final collection runs at the existing completion/abandonment cleanup site.
use super::*;

impl SubagentManager {
    /// At most 256 stopped entries plus `max_concurrent` running entries.
    /// Contexts live inside these entries, so collection drops their authority
    /// and credentials together. Completion alone retains resumability.
    pub const MAX_RETAINED_STOPPED: usize = 256;

    pub(super) fn collect_stopped_agents(&mut self) {
        let mut stopped: Vec<_> = self
            .agents
            .values()
            .filter(|info| info.status != SubagentStatus::Running)
            .map(|info| (info.generation, info.id.clone()))
            .collect();
        stopped.sort_unstable();
        let excess = stopped.len().saturating_sub(Self::MAX_RETAINED_STOPPED);
        for (_, id) in stopped.into_iter().take(excess) {
            self.agents.remove(&id);
            self.parent_ids.remove(&id);
            self.pending_messages.remove(&id);
            self.name_registry.retain(|_, registered| registered != &id);
        }
    }

    pub(crate) fn remember_context(
        &mut self,
        id: &str,
        generation: u64,
        context: std::sync::Arc<runner::EffectiveRunContext>,
    ) -> Result<(), String> {
        let Some(info) = self
            .agents
            .get_mut(id)
            .filter(|info| info.generation == generation)
        else {
            return Err(format!(
                "cannot retain context for agent '{id}': its run was superseded"
            ));
        };
        info.effective_context = Some(context);
        Ok(())
    }
}

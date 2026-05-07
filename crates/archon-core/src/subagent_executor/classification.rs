use super::*;

impl AgentSubagentExecutor {
    pub(super) fn classify_request(&self, request: &SubagentRequest) -> SubagentClassification {
        // Explicit background flag on the request wins.
        if request.run_in_background {
            return SubagentClassification::ExplicitBackground;
        }
        // Agent definition `background: true` cascades to explicit
        // background. Resolving the def requires taking the agent
        // registry read lock; keep this quick (no .await).
        if let Some(ref agent_type) = request.subagent_type
            && let Ok(reg) = self.agent_registry.read()
            && let Some(def) = reg.resolve(agent_type)
            && def.background
        {
            return SubagentClassification::ExplicitBackground;
        }
        // Fork-mode forceAsync pattern: when fork is globally enabled,
        // all agent spawns get forced async. Preserves the old
        // `force_async = is_fork_enabled()` gate at agent.rs:2576-2579.
        if crate::agents::built_in::is_fork_enabled() {
            return SubagentClassification::ExplicitBackground;
        }
        SubagentClassification::Foreground
    }
}

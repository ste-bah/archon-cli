//! Listing live background agents (#189 Phase 9).
//!
//! Until this landed, `BACKGROUND_AGENTS` accepted registrations and could
//! neither be asked what it held nor told to stop anything:
//! `cancel_background_agent` had zero callers, so an `Agent`-tool subagent was
//! uncancellable by model and human alike. The tasks overlay is that caller,
//! and it needs more than the ids `iter_running_ids` returns — it has to show
//! how long each agent has been running, and `spawned_at` lives on the handle.
//!
//! Split from `background_agents.rs` to keep that file under 500 lines.

use std::time::SystemTime;

use dashmap::DashMap;

use super::{AgentId, AgentStatus, BACKGROUND_AGENTS, BackgroundAgentHandle};

/// A live agent, projected for callers that display and cancel them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningAgent {
    /// The registry key: what the runtime, the board and the hooks call it.
    pub subagent_id: String,
    /// The minted id. Equal to `subagent_id` only on the UUID spawn paths.
    pub agent_id: AgentId,
    pub spawned_at: SystemTime,
}

/// Every agent still running.
pub fn running_background_agents() -> Vec<RunningAgent> {
    BACKGROUND_AGENTS.running_snapshot()
}

/// Project the running entries of a registry map.
pub(super) fn snapshot(inner: &DashMap<String, BackgroundAgentHandle>) -> Vec<RunningAgent> {
    inner
        .iter()
        .filter(|entry| entry.current_status() == AgentStatus::Running)
        .map(|entry| RunningAgent {
            subagent_id: entry.key().clone(),
            agent_id: entry.agent_id,
            spawned_at: entry.spawned_at,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_agents::{BackgroundAgentRegistry, BackgroundAgentRegistryApi};
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn handle(subagent_id: &str, status: AgentStatus) -> BackgroundAgentHandle {
        BackgroundAgentHandle {
            agent_id: uuid::Uuid::new_v4(),
            subagent_id: subagent_id.to_string(),
            join_handle: None,
            cancel_token: CancellationToken::new(),
            spawned_at: SystemTime::now(),
            status: Arc::new(Mutex::new(status)),
            result_slot: super::super::new_result_slot(),
        }
    }

    #[test]
    fn only_running_agents_are_listed() {
        let registry = BackgroundAgentRegistry::new();
        registry
            .register(handle("live", AgentStatus::Running))
            .expect("register live");
        registry
            .register(handle("done", AgentStatus::Finished))
            .expect("register finished");

        let running = registry.running_snapshot();

        assert_eq!(running.len(), 1);
        assert_eq!(running[0].subagent_id, "live");
    }

    /// The snapshot must carry the spawn time, which is the whole reason it
    /// exists alongside `iter_running_ids`.
    #[test]
    fn the_snapshot_carries_the_spawn_time() {
        let registry = BackgroundAgentRegistry::new();
        let handle = handle("live", AgentStatus::Running);
        let spawned_at = handle.spawned_at;
        registry.register(handle).expect("register");

        assert_eq!(registry.running_snapshot()[0].spawned_at, spawned_at);
    }

    /// A pipeline agent's runtime id is not its `AgentId`; both must survive
    /// the projection so a caller can tell the two spawn paths apart.
    #[test]
    fn both_identities_survive_the_projection() {
        let registry = BackgroundAgentRegistry::new();
        let handle = handle("session-3-reviewer", AgentStatus::Running);
        let agent_id = handle.agent_id;
        registry.register(handle).expect("register");

        let running = registry.running_snapshot();

        assert_eq!(running[0].subagent_id, "session-3-reviewer");
        assert_eq!(running[0].agent_id, agent_id);
        assert_ne!(running[0].agent_id.to_string(), running[0].subagent_id);
    }

    #[test]
    fn an_empty_registry_yields_no_rows() {
        assert!(BackgroundAgentRegistry::new().running_snapshot().is_empty());
    }
}

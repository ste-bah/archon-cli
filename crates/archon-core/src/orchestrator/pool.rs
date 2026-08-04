use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Default lifetime agent ceiling for a team run.
///
/// Mirrors `WorkflowSpec::default_max_agents` and
/// `archon_topology::GraphBudget::default().max_agents`, so a team and a
/// workflow that declare nothing agree on the ceiling.
pub const DEFAULT_MAX_AGENTS: u32 = 200;

#[derive(Debug)]
pub struct AgentSlot {
    pub agent_id: String,
    pub subtask_id: String,
    pub agent_type: String,
}

/// Two independent ceilings on agent creation.
///
/// `capacity` bounds how many agents may be **live at once** and is released on
/// completion. `lifetime_cap` bounds how many may be **started in total** and is
/// never released.
///
/// The distinction is finding O2: before this, the pool held only `capacity`, so
/// a team that never exceeded its concurrency limit could still start an
/// unbounded number of agents over its run. `GraphBudget::max_agents` is
/// documented as "maximum agents over the graph's whole lifetime — not a
/// concurrency cap", workflows enforce exactly that at fan-out admission, and
/// teams enforced nothing at all.
#[derive(Debug, Clone)]
pub struct AgentPool {
    capacity: u32,
    lifetime_cap: u32,
    active: Arc<Mutex<HashMap<String, AgentSlot>>>,
    /// Lifetime total, never decremented.
    started: Arc<Mutex<u32>>,
}

impl AgentPool {
    /// A pool capping concurrency at `capacity` and lifetime starts at
    /// [`DEFAULT_MAX_AGENTS`].
    pub fn new(capacity: u32) -> Self {
        Self::with_lifetime_cap(capacity, DEFAULT_MAX_AGENTS)
    }

    /// A pool with both ceilings given explicitly.
    pub fn with_lifetime_cap(capacity: u32, lifetime_cap: u32) -> Self {
        Self {
            capacity,
            lifetime_cap,
            active: Arc::new(Mutex::new(HashMap::new())),
            started: Arc::new(Mutex::new(0)),
        }
    }

    /// Whether a concurrency slot is free *right now*.
    ///
    /// A caller polling this must still handle [`AgentPool::acquire`] failing
    /// on the lifetime cap, which no amount of waiting will clear.
    pub async fn can_spawn(&self) -> bool {
        self.active.lock().await.len() < self.capacity as usize
    }

    /// Whether the lifetime budget is exhausted. Waiting will not help.
    pub async fn lifetime_exhausted(&self) -> bool {
        *self.started.lock().await >= self.lifetime_cap
    }

    /// Agents started over this pool's lifetime.
    pub async fn started(&self) -> u32 {
        *self.started.lock().await
    }

    /// Take one concurrency slot and one unit of lifetime budget.
    ///
    /// Both locks are held together so the two checks are one atomic decision;
    /// taking them separately would let two racing spawns each observe room.
    /// Lock order is `active` then `started` here and nowhere else, so there is
    /// no second order to deadlock against.
    pub async fn acquire(
        &self,
        agent_id: String,
        subtask_id: String,
        agent_type: String,
    ) -> anyhow::Result<()> {
        let mut active = self.active.lock().await;
        let mut started = self.started.lock().await;
        if active.len() >= self.capacity as usize {
            anyhow::bail!(
                "agent pool at capacity ({}/{}) — cannot spawn new agent",
                active.len(),
                self.capacity
            );
        }
        if *started >= self.lifetime_cap {
            anyhow::bail!(
                "agent pool lifetime budget exhausted ({}/{}) — cannot spawn a new agent for \
                 subtask '{subtask_id}'; this budget counts every agent ever started, not the \
                 number running now",
                *started,
                self.lifetime_cap
            );
        }
        *started += 1;
        active.insert(
            agent_id.clone(),
            AgentSlot {
                agent_id,
                subtask_id,
                agent_type,
            },
        );
        Ok(())
    }

    /// Release one concurrency slot. The lifetime total stands — that is the
    /// whole point of it.
    pub async fn release(&self, agent_id: &str) {
        self.active.lock().await.remove(agent_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn acquire(pool: &AgentPool, id: &str) -> anyhow::Result<()> {
        pool.acquire(id.to_string(), id.to_string(), "worker".into())
            .await
    }

    #[tokio::test]
    async fn releasing_frees_concurrency_but_not_lifetime_budget() {
        // Finding O2 in one test.
        let pool = AgentPool::with_lifetime_cap(1, 2);

        acquire(&pool, "a").await.expect("first fits");
        pool.release("a").await;
        assert!(pool.can_spawn().await, "concurrency slot was released");

        acquire(&pool, "b")
            .await
            .expect("second fits the lifetime cap");
        pool.release("b").await;
        assert!(pool.can_spawn().await);

        let error = acquire(&pool, "c").await.expect_err("lifetime cap holds");
        assert!(
            error.to_string().contains("lifetime budget exhausted"),
            "{error}"
        );
        assert!(pool.lifetime_exhausted().await);
        assert_eq!(pool.started().await, 2);
    }

    #[tokio::test]
    async fn the_concurrency_cap_still_applies_independently() {
        let pool = AgentPool::with_lifetime_cap(1, 100);
        acquire(&pool, "a").await.expect("first fits");

        let error = acquire(&pool, "b")
            .await
            .expect_err("concurrency cap holds");
        assert!(error.to_string().contains("at capacity"), "{error}");
        assert!(!pool.lifetime_exhausted().await);
    }

    #[tokio::test]
    async fn a_refused_spawn_does_not_consume_lifetime_budget() {
        let pool = AgentPool::with_lifetime_cap(1, 100);
        acquire(&pool, "a").await.expect("first fits");
        let _ = acquire(&pool, "b").await;

        assert_eq!(pool.started().await, 1);
    }
}

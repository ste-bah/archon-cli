use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct AgentSlot {
    pub agent_id: String,
    pub subtask_id: String,
    pub agent_type: String,
}

#[derive(Debug, Clone)]
pub struct AgentPool {
    capacity: u32,
    active: Arc<Mutex<HashMap<String, AgentSlot>>>,
}

impl AgentPool {
    pub fn new(capacity: u32) -> Self {
        Self {
            capacity,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn can_spawn(&self) -> bool {
        self.active.lock().await.len() < self.capacity as usize
    }

    pub async fn acquire(
        &self,
        agent_id: String,
        subtask_id: String,
        agent_type: String,
    ) -> anyhow::Result<()> {
        let mut active = self.active.lock().await;
        if active.len() >= self.capacity as usize {
            anyhow::bail!(
                "agent pool at capacity ({}/{}) — cannot spawn new agent",
                active.len(),
                self.capacity
            );
        }
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

    pub async fn release(&self, agent_id: &str) {
        self.active.lock().await.remove(agent_id);
    }

}

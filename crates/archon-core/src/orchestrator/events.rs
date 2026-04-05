use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtask {
    pub id: String,
    pub description: String,
    pub agent_type: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub status: SubtaskStatus,
    #[serde(default)]
    pub retries: u32,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_max_retries() -> u32 {
    2
}

impl Subtask {
    pub fn new(id: String, description: String, agent_type: String) -> Self {
        Self {
            id,
            description,
            agent_type,
            dependencies: Vec::new(),
            status: SubtaskStatus::Pending,
            retries: 0,
            max_retries: 2,
        }
    }

    pub fn can_retry(&self) -> bool {
        self.retries < self.max_retries
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SubtaskStatus {
    #[default]
    Pending,
    Running,
    Complete {
        result: String,
    },
    Failed {
        error: String,
    },
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum OrchestratorEvent {
    TaskDecomposed {
        subtasks: Vec<Subtask>,
    },
    AgentSpawned {
        agent_id: String,
        agent_type: String,
        subtask_id: String,
    },
    AgentProgress {
        agent_id: String,
        message: String,
    },
    AgentComplete {
        agent_id: String,
        subtask_id: String,
        result: String,
    },
    AgentFailed {
        agent_id: String,
        subtask_id: String,
        error: String,
        will_retry: bool,
    },
    TeamComplete {
        result: String,
    },
    TeamCancelled,
    TeamFailed {
        error: String,
    },
}

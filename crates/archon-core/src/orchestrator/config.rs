use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[default]
    Sequential,
    Parallel,
    Pipeline,
    Dag,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    pub name: String,
    pub coordinator: String,
    pub agents: Vec<String>,
    #[serde(default)]
    pub mode: ExecutionMode,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_max_retries() -> u32 {
    2
}

impl Default for TeamConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            coordinator: String::new(),
            agents: Vec::new(),
            mode: ExecutionMode::default(),
            max_retries: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    /// Agents that may run **at once**.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
    /// Agents that may be started **in total** over one team run.
    ///
    /// Distinct from `max_concurrent`, and new: finding O2 was that `AgentPool`
    /// released its slot on completion and so imposed no lifetime total at all,
    /// leaving a team free to start an unbounded number of agents. The default
    /// matches `WorkflowSpec::default_max_agents`, so a team and a workflow that
    /// declare nothing agree on the ceiling.
    #[serde(default = "default_max_agents")]
    pub max_agents: u32,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_max_concurrent() -> u32 {
    4
}
fn default_max_agents() -> u32 {
    crate::orchestrator::pool::DEFAULT_MAX_AGENTS
}
fn default_timeout_secs() -> u64 {
    300
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            max_agents: default_max_agents(),
            timeout_secs: default_timeout_secs(),
            max_retries: default_max_retries(),
        }
    }
}

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
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_max_concurrent() -> u32 {
    4
}
fn default_timeout_secs() -> u64 {
    300
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            timeout_secs: default_timeout_secs(),
            max_retries: default_max_retries(),
        }
    }
}

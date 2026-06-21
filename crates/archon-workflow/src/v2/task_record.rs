use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2TaskRecord {
    pub task_id: String,
    pub title: String,
    #[serde(default)]
    pub source_paths: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub hard_rules: Vec<String>,
    #[serde(default)]
    pub candidate_target_files: Vec<String>,
    pub status_from_task_file: WorkflowV2TaskFileStatus,
    pub implementation_status: WorkflowV2ImplementationStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowV2TaskFileStatus {
    NotStarted,
    InProgress,
    Blocked,
    Done,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowV2ImplementationStatus {
    Unknown,
    Complete,
    Partial,
    Missing,
    Blocked,
}

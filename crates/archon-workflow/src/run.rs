use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::spec::WorkflowSpec;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Planned,
    Running,
    Paused,
    Failed,
    Cancelled,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Pending,
    Running,
    Paused,
    Accepted,
    Failed,
    Skipped,
    ForcedAccepted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub id: String,
    pub path: PathBuf,
    pub content_hash: String,
    pub producing_stage: String,
    pub source_input_hash: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageState {
    pub id: String,
    pub status: StageStatus,
    pub attempt: u32,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub quality_score: Option<f64>,
    pub artifacts: Vec<ArtifactRef>,
    pub error: Option<String>,
}

impl StageState {
    pub fn pending(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: StageStatus::Pending,
            attempt: 0,
            started_at: None,
            completed_at: None,
            quality_score: None,
            artifacts: Vec::new(),
            error: None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            StageStatus::Accepted
                | StageStatus::Failed
                | StageStatus::Skipped
                | StageStatus::ForcedAccepted
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemState {
    pub id: String,
    pub stage_id: String,
    pub status: StageStatus,
    pub artifact: Option<ArtifactRef>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: String,
    pub spec: WorkflowSpec,
    pub status: RunStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub stages: BTreeMap<String, StageState>,
    pub items: BTreeMap<String, ItemState>,
    pub root: PathBuf,
}

impl WorkflowRun {
    pub fn new(spec: WorkflowSpec, root: impl Into<PathBuf>) -> Self {
        let now = Utc::now();
        let stages = spec
            .stages
            .iter()
            .map(|stage| (stage.id.clone(), StageState::pending(&stage.id)))
            .collect();
        Self {
            id: new_run_id(),
            spec,
            status: RunStatus::Planned,
            created_at: now,
            updated_at: now,
            stages,
            items: BTreeMap::new(),
            root: root.into(),
        }
    }

    pub fn mark_updated(&mut self) {
        self.updated_at = Utc::now();
    }

    pub fn stage_mut(&mut self, id: &str) -> Option<&mut StageState> {
        self.stages.get_mut(id)
    }

    pub fn accepted_stage(&self, id: &str) -> bool {
        self.stages.get(id).is_some_and(|stage| {
            matches!(
                stage.status,
                StageStatus::Accepted | StageStatus::ForcedAccepted
            )
        })
    }
}

pub fn new_run_id() -> String {
    format!("wf-{}", Uuid::new_v4())
}

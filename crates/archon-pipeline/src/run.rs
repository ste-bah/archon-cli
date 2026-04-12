//! Pipeline run state — tracks execution progress of a pipeline instance.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::id::PipelineId;

/// Runtime state of an entire pipeline execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineRun {
    /// Unique identifier for this run.
    pub id: PipelineId,

    /// SHA-256 hex digest of the spec that produced this run.
    pub spec_hash: String,

    /// Current lifecycle state of the pipeline.
    pub state: PipelineState,

    /// Per-step execution state, keyed by step id.
    pub steps: HashMap<String, StepRun>,

    /// When the pipeline run was created.
    pub started_at: DateTime<Utc>,

    /// When the pipeline run reached a terminal state.
    pub finished_at: Option<DateTime<Utc>>,
}

/// Lifecycle state of a pipeline run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineState {
    Pending,
    Running,
    Finished,
    Failed,
    Cancelled,
    RolledBack,
}

/// Runtime state of a single step within a pipeline run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRun {
    /// Task ID from the async task system, if the step has been submitted.
    pub task_id: Option<archon_core::tasks::TaskId>,

    /// Current lifecycle state of this step.
    pub state: StepRunState,

    /// Output produced by the step on success.
    pub output: Option<serde_json::Value>,

    /// Number of execution attempts so far.
    pub attempts: u32,

    /// Error message from the most recent failed attempt.
    pub last_error: Option<String>,
}

/// Lifecycle state of an individual step execution.
///
/// This is a pipeline-local enum (not `archon_core::TaskState`) because
/// pipelines need the `Skipped` variant which the task system does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepRunState {
    Pending,
    Running,
    Finished,
    Failed,
    Cancelled,
    Skipped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_serde_roundtrip() {
        let mut steps = HashMap::new();
        steps.insert(
            "step-1".to_string(),
            StepRun {
                task_id: Some(archon_core::tasks::TaskId::new()),
                state: StepRunState::Finished,
                output: Some(serde_json::json!({"result": 42})),
                attempts: 2,
                last_error: None,
            },
        );
        steps.insert(
            "step-2".to_string(),
            StepRun {
                task_id: None,
                state: StepRunState::Skipped,
                output: None,
                attempts: 0,
                last_error: Some("condition was false".to_string()),
            },
        );

        let run = PipelineRun {
            id: PipelineId::new(),
            spec_hash: "abc123def456".to_string(),
            state: PipelineState::Finished,
            steps,
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
        };

        let json = serde_json::to_string(&run).expect("serialize");
        let deserialized: PipelineRun = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(run, deserialized);
    }
}

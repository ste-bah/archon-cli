use serde::{Deserialize, Serialize};

use crate::events::{WorkflowEvent, WorkflowEventKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTuiEvent {
    pub run_id: String,
    pub agent_name: String,
    pub status: String,
    pub detail: String,
}

impl From<&WorkflowEvent> for WorkflowTuiEvent {
    fn from(event: &WorkflowEvent) -> Self {
        let stage = event
            .detail
            .get("stage")
            .and_then(|value| value.as_str())
            .unwrap_or("workflow");
        Self {
            run_id: event.run_id.clone(),
            agent_name: stage.to_string(),
            status: status_label(&event.kind).to_string(),
            detail: compact_detail(event),
        }
    }
}

fn status_label(kind: &WorkflowEventKind) -> &'static str {
    match kind {
        WorkflowEventKind::Started | WorkflowEventKind::StageStarted => "running",
        WorkflowEventKind::StageCompleted | WorkflowEventKind::Completed => "done",
        WorkflowEventKind::StageStalled => "stalled",
        WorkflowEventKind::StageFailed => "failed",
        // Named explicitly rather than left to the catch-all below, which would
        // label a discarded wave "write_coordination" — the exact class of
        // mislabelling that let a blocking failure read as routine noise.
        WorkflowEventKind::BlockingGapDetected => "blocked",
        WorkflowEventKind::StageSkipped => "skipped",
        WorkflowEventKind::ForcedAccepted => "forced",
        WorkflowEventKind::Resumed => "resumed",
        WorkflowEventKind::Paused => "paused",
        WorkflowEventKind::Cancelled => "cancelled",
        WorkflowEventKind::LearningRecorded => "learning",
        _ => "write_coordination",
    }
}

fn compact_detail(event: &WorkflowEvent) -> String {
    event
        .detail
        .get("status")
        .or_else(|| event.detail.get("error_class"))
        .or_else(|| event.detail.get("name"))
        .and_then(|value| value.as_str())
        .unwrap_or("workflow event")
        .to_string()
}

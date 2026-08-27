//! Live CLI presentation sink for fixed decomposition progress.

use std::sync::Arc;

use archon_workflow::{
    SharedWorkflowUiSink, WorkflowActivityStatus, WorkflowUiEvent, WorkflowUiResult, WorkflowUiSink,
};
use async_trait::async_trait;

pub(crate) struct DecompositionCliUiSink;

impl DecompositionCliUiSink {
    pub(crate) fn shared() -> SharedWorkflowUiSink {
        Arc::new(Self)
    }
}

#[async_trait]
impl WorkflowUiSink for DecompositionCliUiSink {
    async fn emit(&self, event: WorkflowUiEvent) -> WorkflowUiResult {
        match event {
            WorkflowUiEvent::Text(text) => print!("{text}"),
            WorkflowUiEvent::Error(message) => eprintln!("{message}"),
            WorkflowUiEvent::Activity(update) => {
                let status = match update.status {
                    WorkflowActivityStatus::Running => "running",
                    WorkflowActivityStatus::Complete => "complete",
                    WorkflowActivityStatus::Failed => "failed",
                };
                let detail = update
                    .detail
                    .as_deref()
                    .map(|value| format!(": {value}"))
                    .unwrap_or_default();
                eprintln!("[{}] {}{detail}", status, update.name);
            }
        }
        Ok(())
    }
}

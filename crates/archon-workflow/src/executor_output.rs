//! Stage output helpers split out of `executor.rs` to keep that file within the
//! 500-line module budget. These are pure functions over a stage spec / output
//! body with no executor state.

use crate::context;
use crate::error::{WorkflowError, WorkflowResult};
use crate::spec::StageSpec;

/// Render the deterministic (no-live-runner) artifact body for a stage.
///
/// A stage that declares itself a structured fan-out items producer must emit a
/// parseable `items:` document even in the deterministic path, otherwise
/// downstream `foreach` fan-outs would fail-fast with no items.
pub(crate) fn deterministic_stage_output(stage: &StageSpec) -> String {
    if crate::spec::stage_declares_items_producer(stage) {
        if deterministic_empty_items(stage) {
            return r#"{"items":[]}"#.to_string();
        }
        return format!(
            r#"{{"items":[{{"stage":"{}","deterministic":true}}]}}"#,
            stage.id
        );
    }
    format!(
        "# Stage {}\n\nKind: `{:?}`\nAgent: `{}`\n",
        stage.id,
        stage.kind,
        stage.agent.as_deref().unwrap_or("none")
    )
}

fn deterministic_empty_items(stage: &StageSpec) -> bool {
    stage
        .extra
        .get("deterministic_empty_items")
        .or_else(|| stage.input.get("deterministic_empty_items"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Reject a stage output body that self-reports blocked, failed, or
/// unverifiable status before it can be accepted as a usable artifact.
pub(crate) fn ensure_output_usable(body: &str) -> WorkflowResult<()> {
    if let Some(reason) = context::output_reports_blocked(body) {
        return Err(WorkflowError::StageFailed(reason));
    }
    if let Some(reason) = context::output_reports_failed_verification(body) {
        return Err(WorkflowError::StageFailed(reason));
    }
    Ok(())
}

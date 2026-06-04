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

/// Reject a stage output body that self-reports a blocked / missing-evidence
/// status before it can be accepted as a usable artifact.
pub(crate) fn ensure_output_usable(body: &str) -> WorkflowResult<()> {
    if let Some(reason) = context::output_reports_blocked(body) {
        return Err(WorkflowError::StageFailed(reason));
    }
    Ok(())
}

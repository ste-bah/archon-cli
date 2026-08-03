//! What a live run asks the learner, and what it does with the answer.
//!
//! Two functions, split out of `workflow_live` to hold the 500-line ceiling and
//! kept together because they are the two halves of one decision: which
//! evidence bucket this run reads from, and what the run does with what it
//! finds there.

use std::path::Path;

use archon_workflow::{CommandAction, SharedWorkflowUiSink, WorkflowUiEvent};

use super::workflow_live_planner::WorkflowScriptPlan;

/// The task class a run's learned limits are keyed on, or `None` when this
/// action has no task text of its own to classify.
///
/// Only `Plan` and `Run` classify. `RunTemplate` executes a saved spec, which
/// carries `GeneratedWorkflowConfig::default()` by construction, and
/// `Resume`/`Continue` replay the config persisted at creation — a run that
/// changed its repair cap or its timeout halfway through would invalidate the
/// records it had already written under the old one. Both therefore have
/// nothing for the tuner to substitute, and classifying them would only produce
/// a report about a value nobody reads.
pub(super) fn live_task_class(action: &CommandAction) -> Option<&'static str> {
    let task = match action {
        CommandAction::Plan { task } | CommandAction::Run { task, .. } => task,
        _ => return None,
    };
    Some(crate::command::workflow_live_learning_hooks::classify_generated_run(task, None).as_str())
}

/// Resolve this run's learned plan *shape* and attach it to the plan.
///
/// Only decomposed-PRD runs have a shape to resolve: the knob is scored against
/// the plan's own stage families and the declared task graph, and a run with no
/// task universe has neither. A plan without one is left exactly as the planner
/// produced it, which is also what every run got before this existed.
///
/// The report is emitted before any work starts, for the same reason the Phase
/// 7 tuning report is: a user who wonders why this run dispatched two tasks at
/// a time must be able to read the answer in the run's own output rather than
/// reconstruct it from the learning store by hand.
pub(super) async fn apply_generated_shape(
    cwd: &Path,
    class: Option<&str>,
    learning: &archon_core::config::LearningConfig,
    plan: &mut WorkflowScriptPlan,
    ui_sink: &SharedWorkflowUiSink,
) {
    let (Some(class), Some(universe)) = (class, plan.task_universe.as_ref()) else {
        return;
    };
    let tasks_root = crate::command::workflow_live_shape_tuning::tasks_root_of(universe);
    let shape = crate::command::workflow_live_shape_tuning::tune_generated_shape(
        cwd,
        class,
        learning,
        &plan.calls,
        tasks_root.as_deref(),
    );
    let report = shape.report(class);
    // Written onto the plan's own config so every downstream consumer — the
    // lifecycle driver's write fan-out, the persisted metadata, and a resume
    // that replays it — reads the same number. A second substitution point is
    // how a run ends up half-shaped.
    plan.generated_config.implementation_wave_max_parallelism = shape.implementation_wave_width;
    plan.shape_decisions = shape.decisions;
    if report.is_empty() {
        return;
    }
    tracing::info!(class, %report, "generated shape tuned by SONA");
    if let Err(error) = ui_sink.emit(WorkflowUiEvent::Text(report)).await {
        tracing::debug!(%error, "shape report delivery failed");
    }
}

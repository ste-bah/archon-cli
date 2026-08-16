//! `archon world` CLI handlers.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::cli_args::WorldAction;

mod actions;
mod candidate;
/// Coordination outcomes as trace rows (#184 M9).
pub(crate) mod coordination;
mod embedding_runtime;
mod guard;
mod ingest_files;
mod labeling_runtime;
mod predict;
mod runtime;
mod status;
mod surprise_metric;
mod trainer_runtime;

pub(crate) use guard::{
    RuntimeGuardrailRecord, activate_guardrail_for_action, active_guardrail_for_session,
    admit_tool_run_attempt, begin_guarded_action, forced_repair_prompt,
    reclassify_active_guardrail_for_session, record_guardrail_completion_outcome,
    record_guardrail_pipeline_steps, record_guardrail_provider_incident_for_session,
    record_guardrail_reasoning_quality_event, record_guardrail_tool_result_for_session,
    record_guardrail_turn_outcome, record_tool_run_attempt_outcome,
    turn_finalization_verdict_for_action, turn_requirements_for_action,
};
pub(crate) use runtime::{
    record_provider_runtime_advisory, record_runtime_advisory,
    record_runtime_counterfactual_advice, record_runtime_outcome, runtime_prediction_context,
};
pub(super) use status::load_world_model_stats;
pub(crate) use status::render_world_status;
#[cfg(test)]
pub(super) use status::render_world_status_with_stats;
pub(crate) use status::{WorldInspectionRow, world_inspection_rows};
pub(crate) use surprise_metric::{LatentSurpriseContext, record_latent_surprise};
pub(crate) use trainer_runtime::{
    latest_daemon_trainer_event, run_daemon_trainer_tick_controlled_with_activity,
    schedule_dynamic_trainer_tick,
};

pub(crate) fn configure_tool_run_context(
    config: &archon_core::config::ArchonConfig,
    context: &mut archon_tools::tool::ToolContext,
) {
    let admission_config = config.clone();
    context.tool_run_parent_action_id = Some(context.session_id.clone());
    crate::command::topology_admission::install(config, &context.session_id);
    context.tool_run_admission = Some(std::sync::Arc::new(move |request| {
        admit_tool_run_attempt_composed(&admission_config, request)
    }));
    context.tool_run_outcome = Some(std::sync::Arc::new(tool_run_outcome_taps));
}

/// Both admission consumers, in order.
///
/// `ToolRunAdmissionCallback` is a single `Arc<dyn Fn>` with no registry behind
/// it, so a second consumer means composing by hand. **Topology runs first**:
/// it is in-memory and answers in microseconds, while the world-model guardrail
/// persists a candidate trace row and a revision record before it answers.
/// Blocking early skips those writes entirely.
pub(crate) fn admit_tool_run_composed(
    config: &archon_core::config::ArchonConfig,
    request: archon_tools::tool::ToolRunAdmissionRequest,
) -> archon_tools::tool::ToolRunAdmission {
    admit_tool_run_attempt_composed(config, request)
}

fn admit_tool_run_attempt_composed(
    config: &archon_core::config::ArchonConfig,
    request: archon_tools::tool::ToolRunAdmissionRequest,
) -> archon_tools::tool::ToolRunAdmission {
    if let archon_tools::tool::ToolRunAdmission::Blocked { reason } =
        crate::command::topology_admission::admit(&request)
    {
        return archon_tools::tool::ToolRunAdmission::Blocked { reason };
    }
    admit_tool_run_attempt(config, request)
}

/// Fan the tool-run outcome out to all consumers.
///
/// The callback is a single `Arc<dyn Fn>` with no registry behind it, so a
/// second consumer means composing here. Order matters only in that the
/// ambient trace must not be starved by a slow guardrail write; all three are
/// best-effort and none propagates an error.
///
/// The topology release runs before the guardrail write for the same reason
/// admission runs first: it is in-memory, and a spawn's live-agent slot or a
/// write's path claim held any longer than necessary shows up as a false
/// single-writer conflict.
pub(crate) fn tool_run_outcome_taps(outcome: archon_tools::tool::ToolRunAttemptOutcome) {
    crate::command::topology_trace::on_tool_run_outcome(&outcome);
    crate::command::topology_admission::on_tool_run_outcome(&outcome);
    record_tool_run_attempt_outcome(outcome);
}

include!("world_model/root/00_dispatch.rs");
include!("world_model/root/01_helpers.rs");

#[cfg(test)]
mod tests;

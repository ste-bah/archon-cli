//! `archon world` CLI handlers.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::cli_args::WorldAction;

mod actions;
mod candidate;
mod embedding_runtime;
mod guard;
mod ingest_files;
mod labeling_runtime;
mod predict;
mod runtime;
mod status;
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
    context.tool_run_admission = Some(std::sync::Arc::new(move |request| {
        admit_tool_run_attempt(&admission_config, request)
    }));
    context.tool_run_outcome = Some(std::sync::Arc::new(record_tool_run_attempt_outcome));
}

include!("world_model/root/00_dispatch.rs");
include!("world_model/root/01_helpers.rs");

#[cfg(test)]
mod tests;

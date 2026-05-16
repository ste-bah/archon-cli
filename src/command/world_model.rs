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
    RuntimeGuardrailRecord, active_guardrail_for_session, begin_guarded_action,
    forced_repair_prompt, record_guardrail_completion_outcome, record_guardrail_pipeline_steps,
    record_guardrail_provider_incident_for_session, record_guardrail_reasoning_quality_event,
    record_guardrail_tool_result_for_session, record_guardrail_turn_outcome,
};
pub(crate) use runtime::{
    record_provider_runtime_advisory, record_runtime_advisory,
    record_runtime_counterfactual_advice, record_runtime_outcome,
};
pub(super) use status::load_world_model_stats;
pub(crate) use status::render_world_status;
#[cfg(test)]
pub(super) use status::render_world_status_with_stats;
pub(crate) use trainer_runtime::schedule_dynamic_trainer_tick;

include!("world_model/root/00_dispatch.rs");
include!("world_model/root/01_helpers.rs");

#[cfg(test)]
mod tests;

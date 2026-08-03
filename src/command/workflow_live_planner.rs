//! The live planner, and why it is still here.
//!
//! Almost everything in this file used to be a projection between two
//! `archon-workflow` vocabularies — a host call and a stage — and that part
//! moved to [`archon_workflow::v2::plan_metadata`]. What is left is the part
//! that cannot follow it: [`WorkflowScriptPlan`] carries three `archon-core`
//! config types as fields (`GeneratedWorkflowConfig`, and the SONA
//! `GeneratedTuningDecision`/`ShapeDecision` evidence), takes a fourth
//! (`LearningConfig`) to construct, and
//! [`WorkflowScriptPlan::generated`] derives the run's learning hooks through
//! [`crate::command::learning_workflow_hooks`], which classifies the task with
//! archon-topology's `classify_task`.
//!
//! `archon-topology` depends on `archon-workflow`, so that call can never be
//! made from inside `archon-workflow` — it is the exact cycle the crate
//! boundary guard forbids. Carrying the values in a plain struct, the way
//! `LifecycleLimits` carries the generated caps, does not help here either:
//! `GeneratedWorkflowConfig` is not a value crossing one call boundary, it is a
//! field the whole live runtime reads and the run metadata persists.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use archon_core::config::{GeneratedWorkflowConfig, LearningConfig};
use archon_workflow::{
    GeneratedWorkflowLearningContext, ProviderTier, SharedWorkflowUiSink, WorkflowConfig,
    WorkflowGeneratedScaffold, WorkflowLearningEvent, WorkflowLlmClient, WorkflowSpec,
    WorkflowStore, WorkflowUiEvent, WorkflowV2HostCall, workflow_scaffold_hash,
};

use crate::command::learning_workflow_hooks::derive_learning_hooks;

use super::workflow_live_prompt::{harness_planner_prompt, harness_repair_prompt};
use super::workflow_live_retry;
use super::workflow_live_runner::tier_model_alias;
use archon_workflow::repo_root::infer_target_repository_root;
use archon_workflow::task_universe::{
    WorkflowV2TaskUniverse, extract_task_universe_for_generated_run,
};
use archon_workflow::v2::decomposed_prd_plan::{
    decomposed_prd_prompt_slots, decomposed_prd_scaffold,
};
use archon_workflow::v2::lifecycle_driver::LifecycleLimits;
use archon_workflow::v2::plan_metadata::{
    approval_metadata_stage, extract_javascript, workflow_name_from_task,
};

#[derive(Debug, Clone)]
pub(super) struct WorkflowScriptPlan {
    pub(super) name: String,
    pub(super) task: String,
    pub(super) target_repository_root: Option<String>,
    pub(super) max_agents: u32,
    pub(super) max_parallelism: u32,
    pub(super) harness_source: String,
    pub(super) calls: Vec<WorkflowV2HostCall>,
    pub(super) task_universe: Option<WorkflowV2TaskUniverse>,
    pub(super) script_args: Option<serde_json::Value>,
    pub(super) governed_learning_context: Vec<GeneratedWorkflowLearningContext>,
    pub(super) generated_config: GeneratedWorkflowConfig,
    /// The spec's `learning_hooks`, carried through to the persisted run.
    ///
    /// It is the routing selector the learning bridge dispatches on, so a
    /// saved workflow that authored hooks must not lose them here — this
    /// field used to be dropped on the floor, which left the only surface
    /// that can populate it unable to reach its consumer.
    ///
    /// For a *generated* plan it used to be hardcoded empty, which meant no
    /// generated run ever dispatched learning at all. It is now derived from
    /// the run's own content by
    /// [`crate::command::learning_workflow_hooks::derive_learning_hooks`],
    /// at this one construction site so no planner path can forget it. Still
    /// empty when every candidate subsystem is disabled — and empty still
    /// dispatches nothing.
    pub(super) learning_hooks: Vec<String>,
    /// Why this run's generated limits are what they are.
    ///
    /// Empty on every path where SONA did not run, which is the common case and
    /// stays silent. When it is not empty it is persisted with the run metadata
    /// and rendered into the plan, so "why did this run get 5 repair
    /// iterations?" is answerable from the run directory months later — the
    /// learning store holds the evidence, but a run must carry its own reason.
    pub(super) tuning_decisions: Vec<archon_core::config::GeneratedTuningDecision>,
    /// Why this run's plan has the shape it has.
    ///
    /// The structural counterpart of `tuning_decisions`: those explain how long
    /// a stage may run, these explain how work was distributed across branches.
    /// Empty on every path where SONA did not run — and also carries the
    /// decisions where the value did *not* move because a pre-run lint refused
    /// the proposal, which is the case that is otherwise invisible.
    pub(super) shape_decisions: Vec<archon_core::config::ShapeDecision>,
}

impl WorkflowScriptPlan {
    pub(super) fn generated(
        task: &str,
        harness_source: &str,
        calls: Vec<WorkflowV2HostCall>,
        task_universe: Option<WorkflowV2TaskUniverse>,
        generated_config: GeneratedWorkflowConfig,
        learning: &LearningConfig,
    ) -> Self {
        let defaults = WorkflowConfig::default();
        let target_repository_root = infer_target_repository_root(task, task_universe.as_ref());
        let learning_hooks = derive_learning_hooks(task, task_universe.as_ref(), learning);
        Self {
            name: workflow_name_from_task(task),
            task: task.to_string(),
            target_repository_root,
            max_agents: defaults.default_max_agents,
            max_parallelism: defaults.default_max_parallelism,
            harness_source: harness_source.trim().to_string(),
            calls,
            task_universe,
            script_args: None,
            governed_learning_context: Vec::new(),
            generated_config,
            learning_hooks,
            tuning_decisions: Vec::new(),
            shape_decisions: Vec::new(),
        }
    }

    pub(super) fn from_template(
        spec: WorkflowSpec,
        harness_source: &str,
        calls: Vec<WorkflowV2HostCall>,
    ) -> Self {
        Self {
            name: spec.name,
            task: spec.task,
            target_repository_root: spec.target_repository_root,
            max_agents: spec.max_agents,
            max_parallelism: spec.max_parallelism,
            harness_source: harness_source.trim().to_string(),
            calls,
            task_universe: None,
            script_args: None,
            governed_learning_context: Vec::new(),
            generated_config: GeneratedWorkflowConfig::default(),
            learning_hooks: spec.learning_hooks,
            tuning_decisions: Vec::new(),
            shape_decisions: Vec::new(),
        }
    }

    pub(super) fn approval_metadata_spec(&self) -> WorkflowSpec {
        WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: self.name.clone(),
            task: self.task.clone(),
            target_repository_root: self.target_repository_root.clone(),
            max_parallelism: self.max_parallelism,
            max_agents: self.max_agents,
            stages: self
                .calls
                .iter()
                .map(|call| approval_metadata_stage(&self.task, call))
                .collect(),
            permissions: Default::default(),
            learning_hooks: self.learning_hooks.clone(),
        }
    }

    pub(super) fn scaffold_hash(&self) -> String {
        workflow_scaffold_hash(&self.harness_source)
    }

    pub(super) fn generated_scaffold(&self) -> Option<WorkflowGeneratedScaffold> {
        let task_universe = self.task_universe.as_ref()?;
        let task_universe = serde_json::to_value(task_universe).ok()?;
        Some(WorkflowGeneratedScaffold::decomposed_prd(
            self.harness_source.clone(),
            task_universe,
            decomposed_prd_prompt_slots(),
            self.calls.clone(),
            self.governed_learning_context.clone(),
        ))
    }
}

/// Carry the CLI's configured limits across the crate boundary.
///
/// `archon-workflow` does not depend on `archon-core`, so the scaffold takes
/// the driver's own [`LifecycleLimits`] and the conversion happens here — the
/// same boundary shape `run_decomposed_lifecycle` already uses.
pub(super) fn scaffold_limits(generated_config: &GeneratedWorkflowConfig) -> LifecycleLimits {
    LifecycleLimits {
        max_repair_iterations: generated_config.max_repair_iterations,
        max_investigation_iterations: generated_config.max_investigation_iterations,
        implementation_wave_max_parallelism: generated_config.implementation_wave_max_parallelism,
    }
}

#[path = "workflow_live_planner_repair.rs"]
mod workflow_live_planner_repair;
pub(crate) use workflow_live_planner_repair::*;

pub(super) fn render_live_plan(plan: &WorkflowScriptPlan) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!(
        "Workflow V2 harness validated: {} ({} host call(s))\n",
        plan.name,
        plan.calls.len()
    ));
    if plan.task_universe.is_some() {
        out.push_str(&format!(
            "Generated repair caps: max_repair_iterations={}, max_investigation_iterations={}\n",
            plan.generated_config.max_repair_iterations,
            plan.generated_config.max_investigation_iterations
        ));
    }
    for decision in &plan.tuning_decisions {
        if !decision.source.moved() {
            continue;
        }
        out.push_str(&format!(
            "SONA-tuned {}: {} -> {} (weight {:+.4}, {} observation(s))\n",
            decision.parameter.key(),
            decision.baseline,
            decision.applied,
            decision.weight,
            decision.observations
        ));
    }
    // Rendered even when the value did not move, because the two cases that
    // leave it unmoved — a drift rollback and a pre-run lint refusal — are
    // exactly the ones an operator has to know about: the run looks like a
    // default run and is not one by accident.
    for decision in &plan.shape_decisions {
        if !decision.source.noteworthy() {
            continue;
        }
        out.push_str(&format!(
            "SONA-shaped {}: {} -> {} (weight {:+.4}, {} observation(s))\n",
            decision.knob.key(),
            decision.baseline,
            decision.applied,
            decision.weight,
            decision.observations
        ));
        if let Some(refusal) = &decision.refusal {
            out.push_str(&format!("  refused before the run: {refusal}\n"));
        }
    }
    for call in &plan.calls {
        out.push_str(&format!(
            "- {}: w.{} write_mode={:?}\n",
            call.id,
            call.method.as_str(),
            call.write_mode
        ));
    }
    out.push_str("\nworkflow.js:\n");
    out.push_str(&plan.harness_source);
    out.push_str("\n\nworkflow.approval-metadata.yaml:\n");
    out.push_str(&plan.approval_metadata_spec().to_yaml()?);
    Ok(out)
}

#[cfg(test)]
#[path = "workflow_live_planner_tests.rs"]
mod tests;

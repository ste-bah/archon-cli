use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use archon_core::config::{GeneratedWorkflowConfig, LearningConfig};
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_workflow::{
    GeneratedWorkflowLearningContext, ProviderTier, RetryPolicy, StageKind, StageSpec,
    WorkflowConfig, WorkflowGeneratedScaffold, WorkflowLearningEvent, WorkflowLlmClient,
    WorkflowSpec, WorkflowStore, WorkflowV2HostCall, WorkflowV2HostMethod, workflow_scaffold_hash,
};

use crate::command::workflow_live_learning_hooks::derive_learning_hooks;

use super::workflow_live_generated_scaffold::decomposed_prd_scaffold;
use super::workflow_live_prompt::{harness_planner_prompt, harness_repair_prompt};
use super::workflow_live_repo_root::infer_target_repository_root;
use super::workflow_live_retry;
use super::workflow_live_runner::tier_model_alias;
use super::workflow_live_task_universe::{
    WorkflowV2TaskUniverse, extract_task_universe_for_generated_run,
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
    /// [`crate::command::workflow_live_learning_hooks::derive_learning_hooks`],
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
                .map(|call| metadata_stage(&self.task, call))
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

fn decomposed_prd_prompt_slots() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "read_only_discovery".to_string(),
            "Parallel read-only PRD/task/repository/acceptance audits.".to_string(),
        ),
        (
            "implementation_inventory".to_string(),
            "Reducer turns taskUniverse plus discovery into dependency-aware implementation items."
                .to_string(),
        ),
        (
            "implementation_wave".to_string(),
            "Coder fanout receives only dependency-ready readyImplementationItems with coordinated/worktree write mode."
                .to_string(),
        ),
        (
            "remediation".to_string(),
            "Reducer and coder fanout process only non-accepted/non-noop wave outcomes.".to_string(),
        ),
        (
            "verification".to_string(),
            "Focused read-only verification must pass before completedIds unblock dependents."
                .to_string(),
        ),
        (
            "adversarial_review".to_string(),
            "Read-only reducer review and remediation loop check PRD/TASK evidence before final acceptance."
                .to_string(),
        ),
        (
            "final_acceptance".to_string(),
            "Final audit/report receive taskUniverse plus implementation, verification, review, and artifact evidence."
                .to_string(),
        ),
    ])
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

fn metadata_stage(task: &str, call: &WorkflowV2HostCall) -> StageSpec {
    let mut extra = call.options.extra.clone();
    // `condition` is no longer a typed StageSpec field — no evaluator was ever
    // wired up, so it never branched. Leave whatever the plan authored in
    // `extra` so the approval metadata still shows it verbatim.
    strip_reserved_stage_extra(&mut extra);
    StageSpec {
        id: call.id.clone(),
        kind: stage_kind_for_call(call.method),
        task: Some(call.options.task.clone().unwrap_or_else(|| {
            format!(
                "Approval metadata for V2 host call '{}' in generated workflow: {}",
                call.id, task
            )
        })),
        agent: None,
        foreach: None,
        reducer: None,
        tool: declared_tool_name(call),
        depends_on: Vec::new(),
        provider_tier: Some(provider_tier_for_call(call.method)),
        retry: RetryPolicy::default(),
        input: serde_json::json!({
            "runtime": "script_first_v2",
            "metadata_only": true,
            "host_call": call.method.as_str(),
            "write_mode": call.write_mode,
            "source": call.options.source.clone(),
            "role": call.options.role.clone(),
        }),
        model: None,
        provider: None,
        expected_target_files: call.options.target_files.clone(),
        verify_command: None,
        max_parallelism: call.options.max_parallelism.map(|value| value as u32),
        item_kind: call.write_mode.map(|_| StageKind::Implementation),
        filter: None,
        extra,
    }
}

fn declared_tool_name(call: &WorkflowV2HostCall) -> Option<String> {
    if call.method != WorkflowV2HostMethod::Tool {
        return None;
    }
    call.options
        .extra
        .get("tool")
        .or_else(|| call.options.extra.get("name"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn strip_reserved_stage_extra(extra: &mut std::collections::BTreeMap<String, serde_json::Value>) {
    for key in [
        "id",
        "kind",
        "task",
        "agent",
        "foreach",
        "reducer",
        "tool",
        "depends_on",
        "provider_tier",
        "retry",
        "input",
        "model",
        "provider",
        "expected_target_files",
        "verify_command",
        "max_parallelism",
        "item_kind",
        "filter",
    ] {
        extra.remove(key);
    }
}

fn stage_kind_for_call(method: WorkflowV2HostMethod) -> StageKind {
    match method {
        WorkflowV2HostMethod::Agent => StageKind::Agent,
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => StageKind::Fanout,
        WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => StageKind::Reduce,
        WorkflowV2HostMethod::Tool
        | WorkflowV2HostMethod::Checkpoint
        | WorkflowV2HostMethod::SaveArtifact
        | WorkflowV2HostMethod::RequireArtifact => StageKind::Tool,
        WorkflowV2HostMethod::Implementation => StageKind::Implementation,
        WorkflowV2HostMethod::QualityGate => StageKind::QualityGate,
        WorkflowV2HostMethod::HumanGate => StageKind::HumanGate,
    }
}

fn provider_tier_for_call(method: WorkflowV2HostMethod) -> ProviderTier {
    match method {
        WorkflowV2HostMethod::Agent => ProviderTier::Researcher,
        WorkflowV2HostMethod::Fanout
        | WorkflowV2HostMethod::Parallel
        | WorkflowV2HostMethod::Implementation => ProviderTier::Coder,
        WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => ProviderTier::Reducer,
        WorkflowV2HostMethod::QualityGate | WorkflowV2HostMethod::HumanGate => ProviderTier::Critic,
        WorkflowV2HostMethod::Tool
        | WorkflowV2HostMethod::Checkpoint
        | WorkflowV2HostMethod::SaveArtifact
        | WorkflowV2HostMethod::RequireArtifact => ProviderTier::Local,
    }
}

fn workflow_name_from_task(task: &str) -> String {
    let slug = task
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "workflow-v2".to_string()
    } else {
        slug
    }
}

fn extract_javascript(content: &str) -> String {
    let trimmed = content.trim();
    if let Some(start) = trimmed.find("```") {
        let rest = &trimmed[start + 3..];
        let rest = rest
            .strip_prefix("javascript")
            .or_else(|| rest.strip_prefix("js"))
            .unwrap_or(rest);
        let rest = rest.trim_start();
        if let Some(end) = rest.find("```") {
            return rest[..end].trim().to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
#[path = "workflow_live_planner_tests.rs"]
mod tests;

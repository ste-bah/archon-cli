//! Legacy JavaScript-to-`WorkflowSpec` compiler.
//!
//! This module is retained for saved templates and explicit legacy spec/harness
//! flows. Generated `/workflow run <objective>` must use `crate::v2::harness`
//! plus the V2 result store/runtime instead of compiling dynamic harnesses back
//! into YAML-stage execution.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::spec::{
    ArtifactPolicy, ProviderTier, ReducerKind, StageKind, StageSpec, WORKFLOW_SCHEMA, WorkflowSpec,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessPhase {
    pub id: String,
    pub method: String,
    pub kind: StageKind,
    pub depends_on: Vec<String>,
    pub write_capable: bool,
}

#[derive(Debug, Clone, Default)]
pub struct HarnessCompiler;

impl HarnessCompiler {
    pub fn validate(&self, source: &str) -> WorkflowResult<Vec<HarnessPhase>> {
        let executable = executable_source(source);
        reject_unsafe_source(&executable)?;
        let calls = host_calls(&executable)?;
        let mut phases = Vec::new();
        let mut prior_stage = None::<String>;
        let mut variables = BTreeMap::<String, String>::new();
        let mut artifacts = BTreeMap::<String, String>::new();
        for call in calls {
            if matches!(call.method.as_str(), "saveArtifact" | "requireArtifact") {
                continue;
            }
            if call.method == "runCompiledSpec" {
                phases.push(HarnessPhase {
                    id: call.id,
                    method: call.method,
                    kind: StageKind::Checkpoint,
                    depends_on: Vec::new(),
                    write_capable: false,
                });
                continue;
            }
            let kind = method_stage_kind(&call.method)?;
            let stage_kind = call.item_kind.unwrap_or(kind);
            let depends_on = call_depends_on(&call, prior_stage.as_deref(), &variables, &artifacts);
            let phase = HarnessPhase {
                id: call.id.clone(),
                method: call.method.clone(),
                kind,
                depends_on,
                write_capable: stage_kind == StageKind::Implementation,
            };
            prior_stage = Some(phase.id.clone());
            if let Some(variable) = call.variable.clone() {
                variables.insert(variable, phase.id.clone());
            }
            if let Some(artifact) = call.output_artifact.clone() {
                artifacts.insert(artifact, phase.id.clone());
            }
            phases.push(phase);
        }
        if phases.is_empty() {
            return Err(WorkflowError::SpecInvalid(
                "workflow harness declares no executable host calls".to_string(),
            ));
        }
        Ok(phases)
    }

    pub fn compile(&self, source: &str, name: &str, task: &str) -> WorkflowResult<WorkflowSpec> {
        self.validate(source)?;
        let executable = executable_source(source);
        let calls = host_calls(&executable)?;
        let required_artifacts = calls
            .iter()
            .filter(|call| call.method == "requireArtifact")
            .map(|call| call.id.clone())
            .collect::<Vec<_>>();
        let mut prior_stage = None::<String>;
        let mut provider_tiers = BTreeMap::new();
        provider_tiers.insert(ProviderTier::Planner, "auto".to_string());
        provider_tiers.insert(ProviderTier::Researcher, "auto".to_string());
        provider_tiers.insert(ProviderTier::Coder, "auto".to_string());
        provider_tiers.insert(ProviderTier::Critic, "auto".to_string());
        provider_tiers.insert(ProviderTier::Reducer, "auto".to_string());
        let mut stages = Vec::new();
        let mut variables = BTreeMap::<String, String>::new();
        let mut artifacts = BTreeMap::<String, String>::new();
        for call in calls {
            if matches!(
                call.method.as_str(),
                "runCompiledSpec" | "saveArtifact" | "requireArtifact"
            ) {
                continue;
            }
            let kind = method_stage_kind(&call.method)?;
            let depends_on = call_depends_on(&call, prior_stage.as_deref(), &variables, &artifacts);
            if (kind == StageKind::Implementation
                || call.item_kind == Some(StageKind::Implementation))
                && !task_allows_repository_edits(task)
            {
                return Err(WorkflowError::SpecInvalid(format!(
                    "workflow harness declares write-capable stage '{}' for a non-editing task",
                    call.id
                )));
            }
            let stage = stage_for_call(&call, kind, depends_on, task)?;
            prior_stage = Some(stage.id.clone());
            if let Some(variable) = call.variable.clone() {
                variables.insert(variable, stage.id.clone());
            }
            if let Some(artifact) = call.output_artifact.clone() {
                artifacts.insert(artifact, stage.id.clone());
            }
            stages.push(stage);
        }
        if stages.is_empty() {
            return Err(WorkflowError::SpecInvalid(
                "imported compiled-spec wrappers cannot be recompiled without the compiled spec"
                    .to_string(),
            ));
        }
        if !required_artifacts.is_empty() {
            attach_required_artifacts(&mut stages, required_artifacts)?;
        }
        let mut spec = WorkflowSpec {
            schema: WORKFLOW_SCHEMA.to_string(),
            name: sanitize_name(name),
            task: task.to_string(),
            target_repository_root: None,
            max_parallelism: 8,
            max_agents: 200,
            provider_tiers,
            stages,
            artifact_policy: ArtifactPolicy::default(),
            permissions: BTreeMap::new(),
            quality_gates: BTreeMap::new(),
            learning_hooks: Vec::new(),
        };
        ensure_fanout_sources(&mut spec);
        spec.validate()?;
        Ok(spec)
    }
}

include!("harness_host_calls.rs");

include!("harness_stage.rs");

include!("harness_parse_props.rs");

include!("harness_parse_literals.rs");

include!("harness_misc.rs");

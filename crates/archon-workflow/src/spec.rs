use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::generated::normalize_generated_spec;
use crate::spec_deser::{
    deserialize_learning_hooks, deserialize_permissions, deserialize_provider_tiers,
    deserialize_quality_gates,
};

pub const WORKFLOW_SCHEMA: &str = "archon.workflow.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTier {
    Planner,
    Researcher,
    Coder,
    Critic,
    Cheap,
    Vision,
    Local,
    Reducer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Agent,
    Fanout,
    Reduce,
    Condition,
    Tool,
    Checkpoint,
    QualityGate,
    HumanGate,
    Implementation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReducerKind {
    EvidenceWeightedReport,
    ClaimVote,
    AdversarialFindingsMerge,
    CitationReconciliation,
    CodeReviewSynthesis,
    ChapterAssembly,
    TaskDecomposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_base_delay_ms")]
    pub base_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            base_delay_ms: default_base_delay_ms(),
        }
    }
}

fn default_max_attempts() -> u32 {
    1
}

fn default_base_delay_ms() -> u64 {
    1_000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactPolicy {
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_store_agent_outputs")]
    pub store_agent_outputs: bool,
    #[serde(default)]
    pub redact_provider_private_payloads: bool,
}

impl Default for ArtifactPolicy {
    fn default() -> Self {
        Self {
            retention_days: default_retention_days(),
            store_agent_outputs: default_store_agent_outputs(),
            redact_provider_private_payloads: true,
        }
    }
}

fn default_retention_days() -> u32 {
    90
}

fn default_store_agent_outputs() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageSpec {
    pub id: String,
    pub kind: StageKind,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub foreach: Option<String>,
    #[serde(default)]
    pub reducer: Option<ReducerKind>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub provider_tier: Option<ProviderTier>,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// Files an `implementation` stage must mutate for acceptance (Phase B).
    /// Non-empty only for `StageKind::Implementation`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_target_files: Vec<String>,
    /// Shell command that must exit 0 for an `implementation` stage to be
    /// accepted (Phase B acceptance binding).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_command: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSpec {
    pub schema: String,
    pub name: String,
    #[serde(default)]
    pub task: String,
    #[serde(default = "default_max_parallelism")]
    pub max_parallelism: u32,
    #[serde(default = "default_max_agents")]
    pub max_agents: u32,
    #[serde(default, deserialize_with = "deserialize_provider_tiers")]
    pub provider_tiers: BTreeMap<ProviderTier, String>,
    pub stages: Vec<StageSpec>,
    #[serde(default)]
    pub artifact_policy: ArtifactPolicy,
    #[serde(default, deserialize_with = "deserialize_permissions")]
    pub permissions: BTreeMap<String, serde_json::Value>,
    #[serde(default, deserialize_with = "deserialize_quality_gates")]
    pub quality_gates: BTreeMap<String, serde_json::Value>,
    #[serde(default, deserialize_with = "deserialize_learning_hooks")]
    pub learning_hooks: Vec<String>,
}

fn default_max_parallelism() -> u32 {
    8
}

fn default_max_agents() -> u32 {
    200
}

impl WorkflowSpec {
    pub fn from_yaml(input: &str) -> WorkflowResult<Self> {
        let spec: Self = serde_yaml_ng::from_str(input)?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn from_generated_yaml(input: &str, fallback_task: &str) -> WorkflowResult<Self> {
        let mut spec: Self = serde_yaml_ng::from_str(input)?;
        if spec.task.trim().is_empty() {
            spec.task = fallback_task.to_string();
        }
        normalize_generated_spec(&mut spec);
        spec.validate()?;
        Ok(spec)
    }

    pub fn to_yaml(&self) -> WorkflowResult<String> {
        Ok(serde_yaml_ng::to_string(self)?)
    }

    pub fn validate(&self) -> WorkflowResult<()> {
        self.validate_top_level()?;
        self.validate_stage_fields()?;
        self.validate_dependencies()?;
        Ok(())
    }

    fn validate_top_level(&self) -> WorkflowResult<()> {
        if self.schema != WORKFLOW_SCHEMA {
            return Err(WorkflowError::InvalidSchema(self.schema.clone()));
        }
        if self.name.trim().is_empty() {
            return Err(WorkflowError::SpecInvalid("name is required".into()));
        }
        if self.task.trim().is_empty() {
            return Err(WorkflowError::SpecInvalid("task is required".into()));
        }
        if self.max_parallelism == 0 || self.max_agents == 0 {
            return Err(WorkflowError::SpecInvalid(
                "max_parallelism and max_agents must be greater than zero".into(),
            ));
        }
        if self.stages.is_empty() {
            return Err(WorkflowError::SpecInvalid(
                "at least one stage is required".into(),
            ));
        }
        for (tier, value) in &self.provider_tiers {
            if !is_neutral_tier_hint(value) {
                return Err(WorkflowError::HardcodedModel(format!(
                    "provider_tiers.{tier:?}"
                )));
            }
        }
        Ok(())
    }

    fn validate_stage_fields(&self) -> WorkflowResult<()> {
        let mut seen = BTreeSet::new();
        for stage in &self.stages {
            if stage.id.trim().is_empty() {
                return Err(WorkflowError::SpecInvalid("stage id is required".into()));
            }
            if !seen.insert(stage.id.as_str()) {
                return Err(WorkflowError::DuplicateStage(stage.id.clone()));
            }
            if stage.model.as_deref().is_some_and(has_text)
                || stage.provider.as_deref().is_some_and(has_text)
            {
                return Err(WorkflowError::HardcodedModel(stage.id.clone()));
            }
            match stage.kind {
                StageKind::Agent | StageKind::Fanout => {}
                StageKind::Reduce => {}
                StageKind::Condition => require(stage, stage.condition.as_deref(), "condition")?,
                StageKind::Tool => require(stage, stage.tool.as_deref(), "tool")?,
                StageKind::Implementation => {
                    if stage.expected_target_files.iter().all(|f| !has_text(f)) {
                        return Err(WorkflowError::SpecInvalid(format!(
                            "implementation stage '{}' requires at least one expected_target_files entry",
                            stage.id
                        )));
                    }
                }
                StageKind::Checkpoint | StageKind::QualityGate | StageKind::HumanGate => {}
            }
        }
        Ok(())
    }

    fn validate_dependencies(&self) -> WorkflowResult<()> {
        let ids: HashSet<&str> = self.stages.iter().map(|s| s.id.as_str()).collect();
        for stage in &self.stages {
            for dep in &stage.depends_on {
                if dep == &stage.id || !ids.contains(dep.as_str()) {
                    return Err(WorkflowError::UnknownDependency {
                        stage: stage.id.clone(),
                        dependency: dep.clone(),
                    });
                }
            }
        }
        self.detect_cycle()
    }

    fn detect_cycle(&self) -> WorkflowResult<()> {
        let deps: HashMap<&str, Vec<&str>> = self
            .stages
            .iter()
            .map(|s| {
                (
                    s.id.as_str(),
                    s.depends_on.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut stack = Vec::new();
        for stage in &self.stages {
            visit(
                stage.id.as_str(),
                &deps,
                &mut visiting,
                &mut visited,
                &mut stack,
            )?;
        }
        Ok(())
    }
}

fn has_text(value: &str) -> bool {
    !value.trim().is_empty()
}

fn is_neutral_tier_hint(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "auto" | "default" | "inherit" | "active"
    )
}

fn require(stage: &StageSpec, value: Option<&str>, field: &'static str) -> WorkflowResult<()> {
    if value.is_some_and(has_text) {
        return Ok(());
    }
    Err(WorkflowError::MissingStageField {
        stage: stage.id.clone(),
        field,
    })
}

fn visit<'a>(
    node: &'a str,
    deps: &HashMap<&'a str, Vec<&'a str>>,
    visiting: &mut HashSet<&'a str>,
    visited: &mut HashSet<&'a str>,
    stack: &mut Vec<&'a str>,
) -> WorkflowResult<()> {
    if visited.contains(node) {
        return Ok(());
    }
    if !visiting.insert(node) {
        stack.push(node);
        return Err(WorkflowError::DependencyCycle(
            stack.iter().map(|s| (*s).to_string()).collect(),
        ));
    }
    stack.push(node);
    if let Some(children) = deps.get(node) {
        for child in children {
            visit(child, deps, visiting, visited, stack)?;
        }
    }
    stack.pop();
    visiting.remove(node);
    visited.insert(node);
    Ok(())
}

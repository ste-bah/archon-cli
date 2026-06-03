use std::collections::BTreeMap;

use crate::error::WorkflowResult;
use crate::spec::{
    ArtifactPolicy, ProviderTier, ReducerKind, StageKind, StageSpec, WORKFLOW_SCHEMA, WorkflowSpec,
};

pub trait WorkflowPlanner {
    fn plan(&self, task: &str) -> WorkflowResult<WorkflowSpec>;
    fn repair_and_validate(&self, yaml: &str) -> WorkflowResult<WorkflowSpec> {
        WorkflowSpec::from_yaml(yaml)
    }
}

#[derive(Debug, Clone, Default)]
pub struct HeuristicWorkflowPlanner;

impl WorkflowPlanner for HeuristicWorkflowPlanner {
    fn plan(&self, task: &str) -> WorkflowResult<WorkflowSpec> {
        let mut provider_tiers = BTreeMap::new();
        provider_tiers.insert(ProviderTier::Planner, "auto".to_string());
        provider_tiers.insert(ProviderTier::Critic, "auto".to_string());
        provider_tiers.insert(ProviderTier::Reducer, "auto".to_string());
        let spec = WorkflowSpec {
            schema: WORKFLOW_SCHEMA.to_string(),
            name: slug_name(task),
            task: task.to_string(),
            max_parallelism: 8,
            max_agents: 200,
            provider_tiers,
            stages: vec![
                agent(
                    "discover",
                    "workflow-discovery",
                    ProviderTier::Planner,
                    vec![],
                ),
                fanout(
                    "review",
                    "workflow-reviewer",
                    "${discover.items}",
                    ProviderTier::Critic,
                    vec!["discover"],
                ),
                reduce(
                    "synthesize",
                    ReducerKind::EvidenceWeightedReport,
                    ProviderTier::Reducer,
                    vec!["review"],
                ),
                StageSpec {
                    id: "quality".to_string(),
                    kind: StageKind::QualityGate,
                    task: None,
                    agent: None,
                    foreach: None,
                    reducer: None,
                    tool: None,
                    condition: None,
                    depends_on: vec!["synthesize".to_string()],
                    provider_tier: Some(ProviderTier::Critic),
                    retry: Default::default(),
                    input: serde_json::json!({"threshold": 0.50}),
                    model: None,
                    provider: None,
                    expected_target_files: Vec::new(),
                    verify_command: None,
                    extra: BTreeMap::new(),
                },
            ],
            artifact_policy: ArtifactPolicy::default(),
            permissions: BTreeMap::new(),
            quality_gates: BTreeMap::new(),
            learning_hooks: vec!["sona".into(), "reasoning_bank".into(), "world_model".into()],
        };
        spec.validate()?;
        Ok(spec)
    }
}

fn agent(id: &str, agent: &str, tier: ProviderTier, depends_on: Vec<&str>) -> StageSpec {
    StageSpec {
        id: id.to_string(),
        kind: StageKind::Agent,
        task: None,
        agent: Some(agent.to_string()),
        foreach: None,
        reducer: None,
        tool: None,
        condition: None,
        depends_on: depends_on.into_iter().map(str::to_string).collect(),
        provider_tier: Some(tier),
        retry: Default::default(),
        input: serde_json::Value::Object(Default::default()),
        model: None,
        provider: None,
        expected_target_files: Vec::new(),
        verify_command: None,
        extra: BTreeMap::new(),
    }
}

fn fanout(
    id: &str,
    agent_name: &str,
    foreach: &str,
    tier: ProviderTier,
    depends_on: Vec<&str>,
) -> StageSpec {
    let mut stage = agent(id, agent_name, tier, depends_on);
    stage.kind = StageKind::Fanout;
    stage.foreach = Some(foreach.to_string());
    stage
}

fn reduce(id: &str, reducer: ReducerKind, tier: ProviderTier, depends_on: Vec<&str>) -> StageSpec {
    StageSpec {
        id: id.to_string(),
        kind: StageKind::Reduce,
        task: None,
        agent: None,
        foreach: None,
        reducer: Some(reducer),
        tool: None,
        condition: None,
        depends_on: depends_on.into_iter().map(str::to_string).collect(),
        provider_tier: Some(tier),
        retry: Default::default(),
        input: serde_json::Value::Object(Default::default()),
        model: None,
        provider: None,
        expected_target_files: Vec::new(),
        verify_command: None,
        extra: BTreeMap::new(),
    }
}

fn slug_name(task: &str) -> String {
    let slug: String = task
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch.is_whitespace() || ch == '-' || ch == '_' {
                Some('-')
            } else {
                None
            }
        })
        .take(48)
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "dynamic-workflow".to_string()
    } else {
        trimmed.to_string()
    }
}

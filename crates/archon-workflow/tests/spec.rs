use archon_workflow::{ProviderTier, ReducerKind, WorkflowError, WorkflowSpec};

fn valid_yaml() -> &'static str {
    r#"
schema: archon.workflow.v1
name: repo-deep-audit
task: Audit this repository deeply.
max_parallelism: 12
max_agents: 200
provider_tiers:
  planner: auto
  critic: auto
  reducer: auto
stages:
  - id: discover
    kind: agent
    agent: codebase-analyzer
    provider_tier: planner
  - id: review
    kind: fanout
    agent: code-reviewer
    foreach: "${discover.modules}"
    provider_tier: critic
    depends_on: [discover]
  - id: synthesize
    kind: reduce
    reducer: evidence_weighted_report
    depends_on: [review]
"#
}

#[test]
fn spec_roundtrip_identity() {
    let spec = WorkflowSpec::from_yaml(valid_yaml()).unwrap();
    let yaml = spec.to_yaml().unwrap();
    let reparsed = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(spec, reparsed);
}

#[test]
fn unknown_stage_kind_rejected() {
    let bad = valid_yaml().replace("kind: agent", "kind: nope");
    assert!(WorkflowSpec::from_yaml(&bad).is_err());
}

#[test]
fn unknown_dependency_rejected() {
    let bad = valid_yaml().replace("depends_on: [discover]", "depends_on: [missing]");
    let err = WorkflowSpec::from_yaml(&bad).unwrap_err();
    assert!(matches!(err, WorkflowError::UnknownDependency { .. }));
}

#[test]
fn dependency_cycle_rejected() {
    let bad = valid_yaml().replace(
        "provider_tier: planner",
        "provider_tier: planner\n    depends_on: [synthesize]",
    );
    let err = WorkflowSpec::from_yaml(&bad).unwrap_err();
    assert!(matches!(err, WorkflowError::DependencyCycle(_)));
}

#[test]
fn missing_reducer_rejected() {
    let bad = valid_yaml().replace(
        "  - id: synthesize\n    kind: reduce\n    reducer: evidence_weighted_report\n    depends_on: [review]\n",
        "",
    );
    let err = WorkflowSpec::from_yaml(&bad).unwrap_err();
    assert!(matches!(err, WorkflowError::MissingReducer(_)));
}

#[test]
fn hardcoded_model_rejected() {
    let bad = valid_yaml().replace(
        "provider_tier: planner",
        "provider_tier: planner\n    model: sonnet",
    );
    let err = WorkflowSpec::from_yaml(&bad).unwrap_err();
    assert!(matches!(err, WorkflowError::HardcodedModel(_)));
}

#[test]
fn valid_repo_audit_spec_parses() {
    let spec = WorkflowSpec::from_yaml(valid_yaml()).unwrap();
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Critic).unwrap(),
        "auto"
    );
    assert_eq!(
        spec.stages[2].reducer,
        Some(ReducerKind::EvidenceWeightedReport)
    );
}

#[test]
fn learning_hooks_accept_llm_map_shape() {
    let yaml = format!(
        "{}\nlearning_hooks:\n  sona: true\n  reasoning_bank: true\n  reflexion: false\n",
        valid_yaml()
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(spec.learning_hooks, vec!["reasoning_bank", "sona"]);
}

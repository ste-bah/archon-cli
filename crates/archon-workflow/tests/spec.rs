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
    outputs: [items]
  - id: review
    kind: fanout
    agent: code-reviewer
    foreach: "${discover.items}"
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
fn fanout_without_downstream_reducer_is_allowed() {
    let bad = valid_yaml().replace(
        "  - id: synthesize\n    kind: reduce\n    reducer: evidence_weighted_report\n    depends_on: [review]\n",
        "",
    );
    let spec = WorkflowSpec::from_yaml(&bad).unwrap();
    assert_eq!(spec.stages.len(), 2);
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

#[test]
fn learning_hooks_accept_llm_metadata_shapes() {
    let yaml = format!(
        "{}\nlearning_hooks:\n  sona:\n    enabled: true\n    mode: direct\n  reasoning_bank:\n    enabled: true\n  reflexion:\n    enabled: false\n",
        valid_yaml()
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(spec.learning_hooks, vec!["reasoning_bank", "sona"]);
}

#[test]
fn stage_task_is_allowed_for_llm_generated_plans() {
    let yaml = valid_yaml().replace(
        "agent: codebase-analyzer",
        "task: Discover repo modules and summarize implementation risks.\n    agent: codebase-analyzer",
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(
        spec.stages[0].task.as_deref(),
        Some("Discover repo modules and summarize implementation risks.")
    );
}

#[test]
fn stage_extra_metadata_is_preserved_for_llm_generated_plans() {
    let yaml = valid_yaml().replace(
        "agent: codebase-analyzer",
        "agent: codebase-analyzer\n    outputs:\n      - module list\n      - risk summary",
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert!(spec.stages[0].extra.contains_key("outputs"));
}

#[test]
fn agent_name_is_optional_for_generated_agent_stages() {
    let yaml = valid_yaml().replace("    agent: codebase-analyzer\n", "");
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(spec.stages[0].agent, None);
}

#[test]
fn fanout_foreach_is_optional_for_single_item_generated_stages() {
    let yaml = valid_yaml().replace("    foreach: \"${discover.items}\"\n", "");
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(spec.stages[1].foreach, None);
}

#[test]
fn reducer_kind_is_optional_for_generated_reduce_stages() {
    let yaml = valid_yaml().replace("    reducer: evidence_weighted_report\n", "");
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(spec.stages[2].reducer, None);
}

#[test]
fn provider_tiers_accept_llm_map_shape_when_neutral() {
    let yaml = valid_yaml().replace(
        "critic: auto",
        "critic:\n    provider: auto\n    model: auto",
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Critic).unwrap(),
        "auto"
    );
}

#[test]
fn provider_tiers_reject_hardcoded_provider_maps() {
    let yaml = valid_yaml().replace(
        "critic: auto",
        "critic:\n    provider: anthropic\n    model: claude-opus-4-8",
    );
    let err = WorkflowSpec::from_yaml(&yaml).unwrap_err();
    assert!(matches!(err, WorkflowError::HardcodedModel(_)));
}

#[test]
fn provider_tiers_accept_llm_sequence_of_names() {
    let yaml = valid_yaml().replace(
        "provider_tiers:\n  planner: auto\n  critic: auto\n  reducer: auto",
        "provider_tiers:\n  - planner\n  - critic\n  - reducer",
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Planner).unwrap(),
        "auto"
    );
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Critic).unwrap(),
        "auto"
    );
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Reducer).unwrap(),
        "auto"
    );
}

#[test]
fn provider_tiers_accept_llm_sequence_of_single_key_maps() {
    let yaml = valid_yaml().replace(
        "provider_tiers:\n  planner: auto\n  critic: auto\n  reducer: auto",
        "provider_tiers:\n  - planner:\n      provider: auto\n      model: auto\n  - critic: auto\n  - reducer:\n      provider: default",
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Planner).unwrap(),
        "auto"
    );
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Critic).unwrap(),
        "auto"
    );
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Reducer).unwrap(),
        "auto"
    );
}

#[test]
fn provider_tiers_accept_llm_sequence_of_named_maps() {
    let yaml = valid_yaml().replace(
        "provider_tiers:\n  planner: auto\n  critic: auto\n  reducer: auto",
        "provider_tiers:\n  - tier: planner\n    provider: auto\n    model: auto\n  - name: critic\n    value: auto\n  - id: reducer\n    alias: default",
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Planner).unwrap(),
        "auto"
    );
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Critic).unwrap(),
        "auto"
    );
    assert_eq!(
        spec.provider_tiers.get(&ProviderTier::Reducer).unwrap(),
        "default"
    );
}

#[test]
fn provider_tiers_reject_hardcoded_provider_sequence_maps() {
    let yaml = valid_yaml().replace(
        "provider_tiers:\n  planner: auto\n  critic: auto\n  reducer: auto",
        "provider_tiers:\n  - tier: critic\n    provider: anthropic\n    model: claude-opus-4-8",
    );
    let err = WorkflowSpec::from_yaml(&yaml).unwrap_err();
    assert!(matches!(err, WorkflowError::HardcodedModel(_)));
}

#[test]
fn quality_gates_accept_llm_sequence_shape() {
    let yaml = format!(
        "{}\nquality_gates:\n  - id: final-review\n    threshold: 0.8\n  - name: evidence-check\n    require_sources: true\n",
        valid_yaml()
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert!(spec.quality_gates.contains_key("final-review"));
    assert!(spec.quality_gates.contains_key("evidence-check"));
}

#[test]
fn permissions_accept_llm_metadata_values() {
    let yaml = format!(
        "{}\npermissions:\n  default_working_dir: /Volumes/Externalwork/archon-cli/archon-cli\n  allow_writes: true\n  allowed_paths:\n    - /Volumes/Externalwork/archon-cli/archon-cli\n",
        valid_yaml()
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    assert_eq!(
        spec.permissions
            .get("default_working_dir")
            .and_then(serde_json::Value::as_str),
        Some("/Volumes/Externalwork/archon-cli/archon-cli")
    );
    assert_eq!(
        spec.permissions
            .get("allow_writes")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn generated_specs_can_use_fallback_task() {
    let yaml = valid_yaml().replace("task: Audit this repository deeply.\n", "");
    let spec = WorkflowSpec::from_generated_yaml(&yaml, "Fallback task").unwrap();
    assert_eq!(spec.task, "Fallback task");
    assert!(WorkflowSpec::from_yaml(&yaml).is_err());
}

#[test]
fn generated_specs_infer_dependencies_from_io_metadata() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-chain
task: Build a generated chain.
stages:
  - id: discovery
    kind: agent
    outputs: [findings]
  - id: review
    kind: fanout
    inputs: [findings]
    outputs: [reviewed]
  - id: synthesis
    kind: reduce
    inputs: [reviewed]
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    assert_eq!(spec.stages[1].depends_on, vec!["discovery"]);
    assert_eq!(spec.stages[2].depends_on, vec!["review"]);
}

#[test]
fn generated_specs_promote_top_level_quality_gates_to_stages() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-gate
task: Build a generated gate.
stages:
  - id: discovery
    kind: agent
quality_gates:
  final_gate:
    id: final_gate
    task: Check the synthesis before acceptance.
    depends_on: [discovery]
    provider_tier: critic
    criteria:
      - no unsupported claims
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let gate = spec
        .stages
        .iter()
        .find(|stage| stage.id == "final_gate")
        .unwrap();
    assert_eq!(gate.kind, archon_workflow::StageKind::QualityGate);
    assert_eq!(gate.depends_on, vec!["discovery"]);
    assert_eq!(gate.provider_tier, Some(ProviderTier::Critic));
    assert!(gate.extra.contains_key("criteria"));
}

#[test]
fn generated_specs_neutralize_hardcoded_provider_tiers() {
    // Reproduces the live planner failure: the LLM emits a top-level
    // provider_tiers map pinned to a concrete provider/model. That map is
    // never read at runtime, so a generated spec must neutralize it rather
    // than abort with HardcodedModel.
    let yaml = r#"
schema: archon.workflow.v1
name: generated-tiers
task: Implement the decomposed PRD.
provider_tiers:
  planner:
    provider: anthropic
    model: claude-opus-4-8
  researcher: auto
stages:
  - id: discovery
    kind: agent
    provider_tier: planner
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    // Non-neutral entry dropped; neutral hint preserved.
    assert!(!spec.provider_tiers.contains_key(&ProviderTier::Planner));
    assert_eq!(
        spec.provider_tiers
            .get(&ProviderTier::Researcher)
            .map(String::as_str),
        Some("auto")
    );
    // A user-authored spec with the same violation must still be rejected.
    assert!(WorkflowSpec::from_yaml(yaml).is_err());
}

#[test]
fn provider_tiers_skip_unknown_keys_instead_of_aborting() {
    // Reproduces the live failure "unknown provider tier 'hint'": a generated
    // plan emitted a non-tier key. The advisory provider_tiers map must drop
    // unknown keys rather than fail the whole parse.
    let yaml = r#"
schema: archon.workflow.v1
name: generated-unknown-tier
task: Implement the decomposed PRD.
provider_tiers:
  hint: auto
  planner: auto
stages:
  - id: discovery
    kind: agent
    provider_tier: planner
"#;
    let spec = WorkflowSpec::from_yaml(yaml).unwrap();
    assert!(spec.provider_tiers.contains_key(&ProviderTier::Planner));
    assert_eq!(spec.provider_tiers.len(), 1);
}

#[test]
fn generated_specs_normalize_missing_tool_and_condition_stages() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-under-specified
task: Build an underspecified plan.
stages:
  - id: write_progress
    kind: tool
    task: Write compact progress.
  - id: decide_next
    kind: condition
    task: Decide whether more work is needed.
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    assert_eq!(spec.stages[0].kind, archon_workflow::StageKind::Agent);
    assert_eq!(spec.stages[1].kind, archon_workflow::StageKind::Agent);
    assert_eq!(
        spec.stages[0]
            .extra
            .get("normalized_from_kind")
            .and_then(serde_json::Value::as_str),
        Some("Tool")
    );
}

#[test]
fn fanout_over_token_bridges_to_foreach_when_producer_declares_items() {
    // Reproduces the live failure (run wf-f28e3ce2): the planner emitted a
    // decorative `fanout: {over: ordered_workstreams}` block with foreach=null.
    // The generated normalizer must bridge `over` to a real
    // `foreach: ${producer.items}` when the token resolves to an upstream stage
    // that advertises it via `outputs`, and add the depends_on edge.
    let yaml = r#"
schema: archon.workflow.v1
name: generated-fanout-bridge
task: Implement the decomposed PRD.
stages:
  - id: build-dependency-dag
    kind: agent
    provider_tier: planner
    outputs: [task_dag, ordered_workstreams]
  - id: implement-workstreams
    kind: fanout
    provider_tier: coder
    depends_on: [build-dependency-dag]
    fanout:
      over: ordered_workstreams
      respect_dependencies: task_dag
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let fanout = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implement-workstreams")
        .unwrap();
    assert_eq!(
        fanout.foreach.as_deref(),
        Some("${build-dependency-dag.items}")
    );
    assert!(
        fanout
            .depends_on
            .contains(&"build-dependency-dag".to_string())
    );
    // The bridge also records `items` on the producer so the plan satisfies the
    // producer side of the contract end to end.
    let producer = spec
        .stages
        .iter()
        .find(|stage| stage.id == "build-dependency-dag")
        .unwrap();
    let outputs: Vec<String> = match producer.extra.get("outputs") {
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    };
    assert!(outputs.iter().any(|o| o == "items"), "outputs={outputs:?}");
}

#[test]
fn fanout_with_unresolvable_over_token_is_rejected() {
    // A decorative fan-out whose `over` token resolves to no structured-items
    // producer cannot be bridged and must be rejected, not silently collapsed
    // to a single synthetic item at runtime.
    let yaml = r#"
schema: archon.workflow.v1
name: generated-fanout-unresolvable
task: Implement the decomposed PRD.
stages:
  - id: build-dependency-dag
    kind: agent
    provider_tier: planner
  - id: implement-workstreams
    kind: fanout
    provider_tier: coder
    depends_on: [build-dependency-dag]
    fanout:
      over: ordered_workstreams
"#;
    let err = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap_err();
    assert!(
        matches!(err, WorkflowError::InvalidFanout(_)),
        "got {err:?}"
    );
}

#[test]
fn fanout_foreach_without_items_accessor_is_rejected() {
    // A fan-out must iterate via the `.items` accessor; other accessors are
    // never resolved by the runtime.
    let bad = valid_yaml().replace("${discover.items}", "${discover.modules}");
    let err = WorkflowSpec::from_yaml(&bad).unwrap_err();
    assert!(
        matches!(err, WorkflowError::InvalidFanout(_)),
        "got {err:?}"
    );
}

#[test]
fn fanout_foreach_producer_without_items_declaration_is_rejected() {
    // The producer side of the contract: a foreach source that does not declare
    // `outputs: [items]` (or `produces: items`) is rejected.
    let bad = valid_yaml().replace("    outputs: [items]\n", "");
    let err = WorkflowSpec::from_yaml(&bad).unwrap_err();
    assert!(
        matches!(err, WorkflowError::InvalidFanout(_)),
        "got {err:?}"
    );
}

#[test]
fn bare_fanout_without_iteration_remains_valid() {
    // The single-item case: a fan-out with neither foreach nor decorative
    // iteration keys is still a valid spec.
    let yaml = r#"
schema: archon.workflow.v1
name: bare-fanout
task: Single item fanout.
stages:
  - id: discover
    kind: agent
    provider_tier: planner
  - id: review
    kind: fanout
    provider_tier: critic
    depends_on: [discover]
"#;
    let spec = WorkflowSpec::from_yaml(yaml).unwrap();
    assert!(spec.stages[1].foreach.is_none());
}

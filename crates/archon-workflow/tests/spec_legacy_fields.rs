//! Backward compatibility for spec fields that were removed as dead weight.
//!
//! `provider_tiers`, `artifact_policy`, and `quality_gates` were never read, and
//! the `condition` stage kind never branched. They were removed from
//! `WorkflowSpec`/`StageKind`, but persisted state written by earlier builds
//! still carries them: `state.json` embeds a whole `WorkflowSpec`, and saved
//! templates embed the spec YAML. Loading an existing run must keep working.

use archon_workflow::{StageKind, WorkflowRun, WorkflowSpec};

/// A spec exactly as an older build would have serialized it.
const LEGACY_SPEC_YAML: &str = r#"
schema: archon.workflow.v1
name: legacy-run
task: audit the repository
max_parallelism: 8
max_agents: 200
provider_tiers:
  planner: auto
  critic: auto
stages:
  - id: discover
    kind: agent
    agent: workflow-discovery
    outputs: [items]
  - id: gate
    kind: condition
    condition: "plannedItems.length === 0"
    depends_on: [discover]
artifact_policy:
  retention_days: 90
  store_agent_outputs: true
  redact_provider_private_payloads: true
permissions: {}
quality_gates:
  coverage:
    threshold: 0.5
learning_hooks:
  - sona
"#;

#[test]
fn legacy_spec_yaml_still_loads_and_drops_removed_fields() {
    let spec = WorkflowSpec::from_yaml(LEGACY_SPEC_YAML).expect("legacy spec still deserializes");

    assert_eq!(spec.name, "legacy-run");
    assert_eq!(spec.learning_hooks, vec!["sona".to_string()]);

    // Re-serializing must not resurrect the removed keys.
    let yaml = spec.to_yaml().expect("spec serializes");
    assert!(!yaml.contains("provider_tiers"));
    assert!(!yaml.contains("artifact_policy"));
    assert!(!yaml.contains("quality_gates"));
    assert!(
        WorkflowSpec::from_yaml(&yaml).is_ok(),
        "round-tripped spec must still validate"
    );
}

#[test]
fn legacy_condition_stage_loads_as_checkpoint() {
    let spec = WorkflowSpec::from_yaml(LEGACY_SPEC_YAML).expect("legacy spec still deserializes");
    let gate = spec
        .stages
        .iter()
        .find(|stage| stage.id == "gate")
        .expect("condition stage survives the load");

    // A condition stage never branched — no evaluator existed, so it always
    // proceeded. Checkpoint preserves that behaviour exactly.
    assert_eq!(gate.kind, StageKind::Checkpoint);
    // The authored expression is preserved verbatim in the flattened extras.
    assert_eq!(
        gate.extra.get("condition").and_then(|v| v.as_str()),
        Some("plannedItems.length === 0")
    );
}

#[test]
fn legacy_run_state_json_still_loads() {
    // `WorkflowStore::load_state` deserializes WorkflowRun straight from JSON,
    // bypassing `from_yaml`, so the compatibility must live in the Deserialize
    // impl rather than in a YAML pre-pass.
    let spec = WorkflowSpec::from_yaml(LEGACY_SPEC_YAML).expect("legacy spec deserializes");
    let run = WorkflowRun::new(spec, std::path::Path::new("/tmp/legacy"));
    let mut encoded: serde_json::Value =
        serde_json::to_value(&run).expect("run state serializes to JSON");

    // Re-inject the removed keys the way an older build would have written them.
    let spec_obj = encoded["spec"].as_object_mut().expect("spec object");
    spec_obj.insert(
        "provider_tiers".into(),
        serde_json::json!({"planner": "auto"}),
    );
    spec_obj.insert(
        "artifact_policy".into(),
        serde_json::json!({"retention_days": 90}),
    );
    spec_obj.insert("quality_gates".into(), serde_json::json!({"coverage": {}}));

    let decoded: WorkflowRun =
        serde_json::from_value(encoded).expect("legacy run state still deserializes");
    assert_eq!(decoded.spec.name, "legacy-run");
}

#[test]
fn unknown_spec_fields_are_still_rejected() {
    // The compatibility shim must not become a blanket "ignore anything" —
    // typos in hand-authored specs still have to fail loudly.
    let yaml = LEGACY_SPEC_YAML.replace("learning_hooks:", "lerning_hooks:");
    assert!(
        WorkflowSpec::from_yaml(&yaml).is_err(),
        "deny_unknown_fields must still catch typos"
    );
}

use archon_workflow::{
    ProviderTier, StageKind, WorkflowRun, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2HostOptions,
};
use serde_json::json;

use super::WorkflowScriptPlan;

#[test]
fn approval_metadata_round_trips_conditional_host_calls_without_duplicate_fields() {
    let mut options = WorkflowV2HostOptions {
        task: Some("Check whether the dynamic script needs user input.".to_string()),
        ..Default::default()
    };
    options.extra.insert(
        "condition".to_string(),
        json!("plannedItems.length === 0 && proofReview.status !== \"accepted\""),
    );
    options
        .extra
        .insert("runtime_loop".to_string(), json!("while"));
    options.extra.insert(
        "input".to_string(),
        json!("must not flatten over StageSpec.input"),
    );

    let plan = WorkflowScriptPlan::generated(
        "Implement a decomposed PRD",
        "async function workflow(w) { if (plannedItems.length === 0) { await w.humanGate('missing-plan-escalation'); } }",
        vec![WorkflowV2HostCall {
            id: "missing-plan-escalation".to_string(),
            method: WorkflowV2HostMethod::HumanGate,
            write_mode: None,
            options,
        }],
        None,
        archon_core::config::GeneratedWorkflowConfig::default(),
    );

    let spec = plan.approval_metadata_spec();
    let yaml = spec.to_yaml().expect("metadata YAML serializes");
    let parsed_spec =
        archon_workflow::WorkflowSpec::from_yaml(&yaml).expect("metadata YAML parses");
    let stage = parsed_spec
        .stages
        .iter()
        .find(|stage| stage.id == "missing-plan-escalation")
        .expect("metadata stage exists");
    // `condition` is not a typed StageSpec field: nothing ever evaluated it.
    // The authored text still round-trips verbatim through the flattened extras.
    assert_eq!(
        stage.extra.get("condition"),
        Some(&json!(
            "plannedItems.length === 0 && proofReview.status !== \"accepted\""
        ))
    );
    assert_eq!(stage.input["runtime"], "script_first_v2");
    assert_eq!(stage.extra.get("runtime_loop"), Some(&json!("while")));
    assert!(!stage.extra.contains_key("input"));

    let temp = tempfile::tempdir().expect("tempdir");
    let run = WorkflowRun::new(parsed_spec, temp.path());
    let encoded = serde_json::to_string_pretty(&run).expect("run state serializes");
    serde_json::from_str::<WorkflowRun>(&encoded).expect("run state parses");
}

#[test]
fn approval_metadata_surfaces_declared_w_tool_requirements() {
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert("tool".to_string(), json!("requireArtifact"));
    let plan = WorkflowScriptPlan::generated(
        "Inspect declared tool metadata",
        "export default async function workflow(w) { await w.tool('require-final-artifact', { tool: 'requireArtifact' }); }",
        vec![WorkflowV2HostCall {
            id: "require-final-artifact".to_string(),
            method: WorkflowV2HostMethod::Tool,
            write_mode: None,
            options,
        }],
        None,
        archon_core::config::GeneratedWorkflowConfig::default(),
    );

    let spec = plan.approval_metadata_spec();
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "require-final-artifact")
        .expect("tool metadata stage");

    assert_eq!(stage.kind, StageKind::Tool);
    assert_eq!(stage.tool.as_deref(), Some("requireArtifact"));
    assert_eq!(stage.provider_tier, Some(ProviderTier::Local));
}

#[test]
fn a_saved_template_keeps_its_learning_hooks_and_a_generated_plan_has_none() {
    // `learning_hooks` is the learning bridge's routing selector. A saved
    // workflow that authored hooks used to lose them here, which left the only
    // surface that can populate the field unable to reach its consumer.
    let spec = archon_workflow::WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: hooked-template
task: Template with hooks
learning_hooks: [sona, reasoning_bank]
stages:
  - id: a
    kind: agent
    agent: tester
"#,
    )
    .expect("template spec");
    let plan =
        WorkflowScriptPlan::from_template(spec, "export default async function w() {}", Vec::new());
    // The spec deserializer sorts and dedupes hooks, so the authored order is
    // not preserved — only the set is.
    assert_eq!(
        plan.approval_metadata_spec().learning_hooks,
        vec!["reasoning_bank".to_string(), "sona".to_string()]
    );

    // Nothing authored a hook for a generated plan, so nothing dispatches.
    let generated = WorkflowScriptPlan::generated(
        "Implement a decomposed PRD",
        "export default async function w() {}",
        Vec::new(),
        None,
        archon_core::config::GeneratedWorkflowConfig::default(),
    );
    assert!(generated.approval_metadata_spec().learning_hooks.is_empty());
}

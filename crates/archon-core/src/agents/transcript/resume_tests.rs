//! Sidecars never contribute authority, and refusals name the recovery.
use super::*;

#[test]
fn an_unknown_context_refusal_names_the_agent_and_the_recovery() {
    assert_eq!(
        unknown_context("child"),
        "cannot continue agent 'child': its confinement is only known to the process that started it; start a new agent"
    );
}

#[test]
fn sidecar_retains_only_the_original_descriptive_fields() {
    let metadata = super::super::AgentMetadata {
        agent_type: "worker".into(),
        worktree_path: Some("/work".into()),
        description: Some("task".into()),
        filename: Some("worker.md".into()),
    };
    let json = serde_json::to_value(metadata).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 4);
    assert!(json.get("confinement").is_none());
    let legacy = serde_json::json!({"agent_type":"worker","confinement":"invalid"});
    let parsed: super::super::AgentMetadata = serde_json::from_value(legacy).unwrap();
    assert_eq!(parsed.agent_type, "worker");
}

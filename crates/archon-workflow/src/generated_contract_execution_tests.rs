use super::*;
use crate::task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};

fn task(id: &str, verifier: Option<&str>) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        source_path: format!("/tmp/{id}.md"),
        deliverable_contracts: vec![WorkflowV2DeliverableContract {
            kind: "dataset".to_string(),
            artifact_path: "data/registry.json".to_string(),
            typed_verifier_command: verifier.map(str::to_string),
            registry_path: None,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn universe(tasks: Vec<WorkflowV2TaskUniverseTask>) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp".to_string()],
        tasks,
    }
}

fn noop_item(task_id: &str, commands_run: Option<serde_json::Value>) -> serde_json::Value {
    let mut item = serde_json::json!({
        "item_id": "noop-1",
        "work_type": "verified_noop",
        "canonical_task_ids": [task_id],
        "acceptance_criteria": ["the ingest leaves persisted datasets"],
        "noop_proof": "the artifact files already exist",
        "noop_proof_refs": ["crates/x/src/ingest.rs"],
        "artifact_requirements": ["data/registry.json"],
    });
    if let Some(commands) = commands_run {
        item["commands_run"] = commands;
    }
    item
}

fn issue_fields(issues: &[GeneratedContractIssue]) -> Vec<String> {
    issues.iter().map(|issue| issue.field.clone()).collect()
}

/// The live failure. An ingest task declares a command that must run; the agent
/// answers "the files are already there" and the registry stays empty.
#[test]
fn a_noop_is_refused_when_the_contract_declares_a_command_to_run() {
    let contract = ContractTaskUniverse::from_authoritative(Some(&universe(vec![task(
        "TASK-A-001",
        Some("cargo run -- ingest --verify"),
    )])));

    let issues = generated_item_issues(&noop_item("TASK-A-001", None), &contract, None);

    assert!(
        issue_fields(&issues).contains(&"commands_run".to_string()),
        "a no-op on an execution task must be refused: {issues:?}"
    );
}

/// Recording the command that ran is the whole point — evidence of execution,
/// not of existence. With it, the no-op is allowed.
#[test]
fn a_noop_with_recorded_commands_is_allowed() {
    let contract = ContractTaskUniverse::from_authoritative(Some(&universe(vec![task(
        "TASK-A-001",
        Some("cargo run -- ingest --verify"),
    )])));

    let issues = generated_item_issues(
        &noop_item(
            "TASK-A-001",
            Some(serde_json::json!(["cargo run -- ingest --verify"])),
        ),
        &contract,
        None,
    );

    assert!(
        !issue_fields(&issues).contains(&"commands_run".to_string()),
        "recorded execution must satisfy the rule: {issues:?}"
    );
}

/// A task with no command to run is unaffected — docs and specs are finished by
/// inspection, and this rule must not make every no-op illegal.
#[test]
fn a_task_without_a_verifier_command_still_allows_a_noop() {
    let contract =
        ContractTaskUniverse::from_authoritative(Some(&universe(vec![task("TASK-A-002", None)])));

    let issues = generated_item_issues(&noop_item("TASK-A-002", None), &contract, None);

    assert!(
        !issue_fields(&issues).contains(&"commands_run".to_string()),
        "a non-execution task keeps its no-op: {issues:?}"
    );
}

/// With no authoritative universe there is nothing to judge against, so the
/// rule stays silent rather than guessing.
#[test]
fn an_absent_task_universe_does_not_refuse_anything() {
    let contract = ContractTaskUniverse::from_authoritative(None);

    let issues = generated_item_issues(&noop_item("TASK-A-001", None), &contract, None);

    assert!(!issue_fields(&issues).contains(&"commands_run".to_string()));
}

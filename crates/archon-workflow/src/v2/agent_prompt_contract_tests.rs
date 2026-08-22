//! The gate that decides who is shown a task's declared contract, and the
//! scoping of what they are shown.

use super::contract::insert_task_contract_context;
use super::uses_task_contract_context;
use crate::v2::WorkflowV2HostMethod;

fn universes() -> Vec<serde_json::Value> {
    vec![serde_json::json!({
        "tasks": [
            {"canonical_task_id": "TASK-A-010", "acceptance_criteria": ["a1", "a2"]},
            {"canonical_task_id": "TASK-A-020", "aliases": ["TDL-020"], "acceptance_criteria": ["b1"]},
            {"canonical_task_id": "TASK-A-030", "acceptance_criteria": ["c1"]},
        ]
    })]
}

fn contract_ids(invocation: &serde_json::Value) -> Vec<String> {
    invocation["task_contract_context"]["tasks"]
        .as_array()
        .expect("tasks array")
        .iter()
        .map(|t| {
            t["canonical_task_id"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

/// The defect: the dialect labels its remediation calls `remediate-`, the gate
/// tested `remediation-`, and so the agents re-running a task after a verifier
/// rejected it never saw the criteria they had failed.
#[test]
fn a_remediation_call_is_shown_the_contract_whatever_it_is_labelled() {
    for call_id in [
        "remediate-tdl-020-1-0",
        "implement-tdl-020-3-0",
        "some-label-the-author-invented-0",
    ] {
        assert!(
            uses_task_contract_context(WorkflowV2HostMethod::Implementation, call_id),
            "{call_id} was not shown its contract"
        );
    }
}

/// The method is the host's own; the label is the script author's. A call that
/// is not write-capable and names nothing still must not be swept in.
#[test]
fn an_ordinary_read_only_call_is_not_shown_the_contract() {
    assert!(!uses_task_contract_context(
        WorkflowV2HostMethod::Agent,
        "author-workflow-script"
    ));
    assert!(!uses_task_contract_context(
        WorkflowV2HostMethod::Agent,
        "inventory-1"
    ));
}

#[test]
fn the_roles_that_were_already_covered_stay_covered() {
    assert!(uses_task_contract_context(
        WorkflowV2HostMethod::FinalReport,
        "final-report"
    ));
    for call_id in ["verification-wave-2", "adversarial-review-1-map"] {
        assert!(
            uses_task_contract_context(WorkflowV2HostMethod::Agent, call_id),
            "{call_id} lost its contract context"
        );
    }
}

/// Scoping. A write agent asked to satisfy one task must not have its own
/// contract buried under every other task's.
#[test]
fn the_contract_is_scoped_to_the_tasks_the_call_claims() {
    let mut invocation = serde_json::json!({"item": {"canonical_task_ids": ["TASK-A-020"]}});
    insert_task_contract_context(&mut invocation, &universes(), &["TASK-A-020".to_string()]);
    assert_eq!(contract_ids(&invocation), vec!["TASK-A-020"]);
}

/// A call claiming nothing — a reducer surveying the run — still gets the lot,
/// because that is genuinely its subject.
#[test]
fn a_call_claiming_nothing_is_shown_every_task() {
    let mut invocation = serde_json::json!({});
    insert_task_contract_context(&mut invocation, &universes(), &[]);
    assert_eq!(
        contract_ids(&invocation),
        vec!["TASK-A-010", "TASK-A-020", "TASK-A-030"]
    );
}

/// An id that resolves to no task falls back to everything. Starving an agent
/// of its contract is the failure this path exists to prevent, so an
/// unresolvable claim must not be the way it happens.
#[test]
fn an_unresolvable_claim_falls_back_rather_than_starving_the_agent() {
    let mut invocation = serde_json::json!({});
    insert_task_contract_context(&mut invocation, &universes(), &["TASK-NOPE".to_string()]);
    assert_eq!(contract_ids(&invocation).len(), 3);
}

/// Aliases resolve too — a task claimed by the short id it is also known by is
/// the same task.
#[test]
fn a_claim_by_alias_resolves() {
    let mut invocation = serde_json::json!({});
    insert_task_contract_context(&mut invocation, &universes(), &["TDL-020".to_string()]);
    assert_eq!(contract_ids(&invocation), vec!["TASK-A-020"]);
}

/// And the criteria actually arrive, not just the task ids.
#[test]
fn the_scoped_contract_carries_the_acceptance_criteria() {
    let mut invocation = serde_json::json!({});
    insert_task_contract_context(&mut invocation, &universes(), &["TASK-A-010".to_string()]);
    assert_eq!(
        invocation["task_contract_context"]["tasks"][0]["acceptance_criteria"],
        serde_json::json!(["a1", "a2"])
    );
}

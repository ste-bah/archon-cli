//! The gate that decides who is shown a task's declared contract, and the
//! scoping of what they are shown.

use super::contract::insert_task_contract_context;
use super::contract_gate::uses_task_contract_context as gate;
use crate::v2::{WorkflowV2HostCall, WorkflowV2HostMethod};

fn call(method: WorkflowV2HostMethod, id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method,
        write_mode: None,
        options: Default::default(),
    }
}

fn uses_task_contract_context(method: WorkflowV2HostMethod, id: &str) -> bool {
    gate(&call(method, id), id)
}

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
    for call_id in [
        "verification-wave-2",
        "adversarial-review-1-map",
        "cross-cutting-review",
        "review-remediation-inventory-3",
        "artifact-inventory",
        "artifact-existence-investigation-1-x",
    ] {
        assert!(
            uses_task_contract_context(WorkflowV2HostMethod::Agent, call_id),
            "{call_id} lost its contract context"
        );
    }
}

/// Issue-216: a task whose id holds `review`, `artifact`, `verification` or
/// `completion-evidence` must get the same gate decision as one whose id does
/// not, for the same kind of call.
#[test]
fn a_task_named_review_or_artifact_does_not_flip_the_gate() {
    for label in [
        "inventory-tdl-adversarial-review-020-1",
        "summarise-task-artifact-store-2",
        "inventory-tdl-data-verification-020-1",
        "summarise-task-completion-evidence-store-2",
        "scan-tdl-020-1",
    ] {
        assert!(
            !uses_task_contract_context(WorkflowV2HostMethod::Agent, label),
            "{label}: a task's name turned the gate on"
        );
        assert!(
            uses_task_contract_context(WorkflowV2HostMethod::Implementation, label),
            "{label}: an implementation lost its contract"
        );
    }
}

/// Issue-216: the verifier and completion-evidence stages the engine itself
/// names, which declare no item kind, are still shown the contract: they are
/// recognised by the stage name the id STARTS with, not by the word anywhere.
#[test]
fn the_engine_verification_stages_are_shown_the_contract() {
    for call_id in [
        "verification-plan-3",
        "verification-failure-triage-2-1",
        "verification-remediation-inventory-1-2",
        "post-remediation-verification-plan-1-2",
        "noop-proof-verification-1",
        "noop-proof-reverification-1-2",
        "wave-completion-evidence-repair-2",
        "review-verification-plan-1",
        "remediation-wave-1-verification-2",
    ] {
        assert!(
            uses_task_contract_context(WorkflowV2HostMethod::Reduce, call_id),
            "{call_id} lost its contract context"
        );
    }
}

/// The role the host holds decides, whatever the author labelled the call.
#[test]
fn a_declared_role_is_shown_the_contract_whatever_its_label() {
    for kind in ["focused_verification", "review_map", "noop_proof"] {
        let mut c = call(WorkflowV2HostMethod::Agent, "anything-1");
        c.options.item_kind = Some(kind.to_string());
        assert!(gate(&c, "anything-1"), "{kind} lost its contract");
    }
    for key in ["reviewContract", "remediationContract"] {
        let mut c = call(WorkflowV2HostMethod::Reduce, "anything-2");
        c.options
            .extra
            .insert(key.to_string(), serde_json::json!({"version": 1}));
        assert!(gate(&c, "anything-2"), "{key} lost its contract");
    }
    let mut c = call(WorkflowV2HostMethod::Agent, "anything-3");
    c.options.item_kind = Some("inventory".to_string());
    assert!(!gate(&c, "anything-3"));
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

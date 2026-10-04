//! Shared fixtures for the restart branch-revocation tests.
#![allow(dead_code, unused_imports)]

use super::restart_run::{generated_run, v2_store};
use archon_workflow::v2::branch_cache::split_reusable_branch_outcomes;
use archon_workflow::v2::restart::{invalidate_generated_v2_item, restart_generated_v2_task};
use archon_workflow::v2::reuse_identity::reuse_identity;
use archon_workflow::{
    WorkflowV2BranchOutcome, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Status, WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind,
    WorkflowV2WriteMode,
};

pub const CALL: &str = "implementation-wave-1";

/// The write branch of `task` in the wave, as the host derives it.
pub fn item(task: &str) -> WorkflowV2FanoutItem {
    let id = format!("{CALL}-{task}");
    let call = WorkflowV2HostCall {
        id: id.clone(),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: Default::default(),
    };
    WorkflowV2FanoutItem::read_only(
        id,
        "coder",
        call,
        serde_json::json!({
            "fanout_call_id": CALL,
            "fanout_item_id": task,
            "item": { "id": task, "canonical_task_ids": [task], "target_files": ["src/lib.rs"] },
        }),
    )
}

/// An outcome of `task`'s branch at `status`, recorded with `hash`.
pub fn outcome(
    task: &str,
    status: WorkflowV2Status,
    hash: &str,
    landed: bool,
) -> WorkflowV2BranchOutcome {
    let branch = item(task);
    let mut result = WorkflowV2Result::accepted("branch produced the declared change");
    result.status = status;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "branch recorded concrete implementation evidence",
    ));
    result.data = serde_json::json!({
        "branch_id": branch.id,
        "canonical_task_ids": [task],
        "patch_landed": landed,
    });
    WorkflowV2BranchOutcome {
        item_id: branch.id.clone(),
        role: "coder".to_string(),
        status,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: Some(hash.to_string()),
        completion_evidence: vec![WorkflowV2TaskCompletionEvidence::new(
            task,
            WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate,
            CALL,
            &branch.id,
            status,
        )],
    }
}

/// `task` landed (accepted, `patch_landed`, hash H1), then a later save with
/// another hash (a replay's no-op, H2) superseded that record.
pub fn landed_then_superseded(v2: &WorkflowV2ResultStore, task: &str) {
    v2.save_branch_outcome(CALL, &outcome(task, WorkflowV2Status::Accepted, "H1", true))
        .unwrap();
    v2.save_branch_outcome(CALL, &outcome(task, WorkflowV2Status::Noop, "H2", false))
        .unwrap();
    let archive = v2.branch_outcome_path(CALL, &item(task).id);
    let archive = archive.parent().unwrap().join("superseded");
    assert_eq!(
        std::fs::read_dir(archive).unwrap().count(),
        1,
        "H1 archived"
    );
}

/// `(reused, pending)` item ids for one split of `tasks`.
pub fn split(v2: &WorkflowV2ResultStore, tasks: &[&str]) -> (Vec<String>, Vec<String>) {
    let items = tasks.iter().map(|task| item(task)).collect();
    let (reused, pending) = split_reusable_branch_outcomes(v2, CALL, items).unwrap();
    (
        reused.into_iter().map(|outcome| outcome.item_id).collect(),
        pending.into_iter().map(|item| item.id).collect(),
    )
}

pub fn assert_revoked(v2: &WorkflowV2ResultStore, task: &str) {
    let branch = item(task).id;
    let (reused, pending) = split(v2, &[task]);
    assert!(
        reused.is_empty(),
        "{task}: revoked outcome reused: {reused:?}"
    );
    assert_eq!(pending, vec![branch.clone()]);
    assert!(
        !v2.branch_outcome_path(CALL, &branch).exists(),
        "{task}: the branch cache wrote a revoked outcome back into the slot"
    );
}

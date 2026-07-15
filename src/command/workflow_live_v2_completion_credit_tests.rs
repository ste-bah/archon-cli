use archon_workflow::{
    WorkflowV2BranchOutcome, WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind,
};

use super::workflow_live_task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use super::workflow_live_v2_completion_credit::{CompletionCredit, prepare_resume_credit};

#[test]
fn resume_credit_combines_noop_and_verified_implementation_branches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    save_outcome(&store, "implementation", noop_evidence("TASK-ONE"));
    save_outcome(
        &store,
        "implementation",
        implementation_evidence("TASK-TWO"),
    );
    save_outcome(&store, "verification", verification_evidence("TASK-TWO"));

    let completed = CompletionCredit::from_store(&store)
        .expect("credit")
        .completed_ids();

    assert_eq!(completed, set(&["TASK-ONE", "TASK-TWO"]));
}

#[test]
fn resume_credit_excludes_blocked_terminal_claims_and_archives_terminals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    save_terminal(
        &store,
        "blocked-old",
        include_str!("fixtures/wfb36_resume_completion_credit.json"),
        "2026-07-12T14:56:52Z",
    );
    save_terminal(
        &store,
        "blocked-new",
        include_str!("fixtures/wfb36_resume_missing_credit.json"),
        "2026-07-12T17:00:04Z",
    );

    let completed = prepare_resume_credit(&store, &universe()).expect("resume credit");

    assert_eq!(completed, set(&["TASK-ONE", "TASK-TWO", "TASK-BLOCKED"]));
    assert!(
        store
            .load_call_record("blocked-old")
            .expect("lookup")
            .is_none()
    );
    let archive = store.root().join("archived-resume-terminals");
    assert_eq!(files_below(&archive), 2);
}

fn save_outcome(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    evidence: WorkflowV2TaskCompletionEvidence,
) {
    let outcome = WorkflowV2BranchOutcome {
        item_id: evidence.item_id.clone(),
        role: "worker".to_string(),
        status: evidence.status,
        result: None,
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: vec![evidence],
    };
    store
        .save_branch_outcome(call_id, &outcome)
        .expect("outcome");
}

fn save_terminal(store: &WorkflowV2ResultStore, call_id: &str, fixture: &str, finished_at: &str) {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        data: serde_json::from_str(fixture).expect("D28 terminal fixture"),
        ..WorkflowV2Result::default()
    };
    let call = WorkflowV2HostCall {
        id: call_id.to_string(),
        method: WorkflowV2HostMethod::FinalReport,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    let mut record = WorkflowV2CallRecord::new(
        store.run_id(),
        call,
        1,
        "input".to_string(),
        result,
        Vec::new(),
    );
    record.finished_at = finished_at.to_string();
    store.save_call_record(&record).expect("terminal record");
}

fn noop_evidence(task_id: &str) -> WorkflowV2TaskCompletionEvidence {
    evidence(
        task_id,
        WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate,
        WorkflowV2Status::Noop,
    )
}

fn implementation_evidence(task_id: &str) -> WorkflowV2TaskCompletionEvidence {
    evidence(
        task_id,
        WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate,
        WorkflowV2Status::Accepted,
    )
}

fn verification_evidence(task_id: &str) -> WorkflowV2TaskCompletionEvidence {
    evidence(
        task_id,
        WorkflowV2TaskCompletionEvidenceKind::FocusedVerification,
        WorkflowV2Status::Accepted,
    )
}

fn evidence(
    task_id: &str,
    kind: WorkflowV2TaskCompletionEvidenceKind,
    status: WorkflowV2Status,
) -> WorkflowV2TaskCompletionEvidence {
    WorkflowV2TaskCompletionEvidence::new(task_id, kind, "call", format!("item-{task_id}"), status)
}

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["tasks".to_string()],
        tasks: ["TASK-ONE", "TASK-TWO", "TASK-BLOCKED", "TASK-MISSING"]
            .into_iter()
            .map(|id| WorkflowV2TaskUniverseTask {
                canonical_task_id: id.to_string(),
                aliases: Vec::new(),
                source_path: format!("tasks/{id}.md"),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            })
            .collect(),
    }
}

fn set(ids: &[&str]) -> std::collections::BTreeSet<String> {
    ids.iter().map(|id| (*id).to_string()).collect()
}

fn files_below(root: &std::path::Path) -> usize {
    std::fs::read_dir(root)
        .expect("archive root")
        .flat_map(|entry| std::fs::read_dir(entry.expect("attempt").path()).expect("attempt dir"))
        .count()
}

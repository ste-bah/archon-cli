//! The learning-hook routing half of the fold (L3).
//!
//! `learning_hooks` is the routing selector, so the assertions here are about
//! what a hook list does and does not cause: an empty list must dispatch
//! nothing, a hook with no consumer must be reported rather than silently
//! dropped, and a stage that never finished must not be attributed.

use archon_workflow::{
    StageStatus, Verification, WorkflowLearningRecord, WorkflowStore, learning::StageTelemetry,
};

use super::super::workflow_learning::{fold_workflow_learning, plan_dispatch};

fn record(stage_id: &str, verification: Verification, hooks: &[&str]) -> WorkflowLearningRecord {
    WorkflowLearningRecord {
        run_id: "run-1".into(),
        name: "demo".into(),
        stage_id: stage_id.into(),
        phase: "reduce".into(),
        agent: None,
        status: match verification {
            Verification::Accepted => StageStatus::Accepted,
            Verification::Forced => StageStatus::ForcedAccepted,
            Verification::Failed => StageStatus::Failed,
            Verification::Unverified => StageStatus::Pending,
        },
        verification,
        durable: false,
        quality_score: None,
        artifact_refs: Vec::new(),
        telemetry: StageTelemetry {
            attempt: 1,
            error_class: None,
            artifact_count: 0,
        },
        trace_ref: None,
        hooks: hooks.iter().map(|hook| (*hook).to_string()).collect(),
        ts: chrono::Utc::now(),
    }
}

#[test]
fn empty_hook_list_dispatches_nothing() {
    let plan = plan_dispatch(&[record("a", Verification::Accepted, &[])]);
    assert!(plan.calls.is_empty());
    assert_eq!(plan.skipped_unhooked, 1);
    assert!(plan.unrouted_hooks.is_empty());
}

#[test]
fn only_hooks_with_a_consumer_dispatch() {
    let plan = plan_dispatch(&[
        record("a", Verification::Accepted, &["sona"]),
        record("b", Verification::Accepted, &["world_model"]),
    ]);
    assert_eq!(plan.calls.len(), 1);
    assert_eq!(plan.calls[0].agent, "reduce");
    assert_eq!(plan.skipped_unhooked, 1);
    // A hook with no write-side entry point is reported, not dropped.
    assert!(plan.unrouted_hooks.contains("worldmodel"));
}

#[test]
fn hook_spelling_is_normalized() {
    for spelling in ["reasoning_bank", "reasoning-bank", "ReasoningBank"] {
        let plan = plan_dispatch(&[record("a", Verification::Accepted, &[spelling])]);
        assert_eq!(plan.calls.len(), 1, "{spelling} should route");
        assert!(plan.unrouted_hooks.is_empty(), "{spelling}");
    }
}

#[test]
fn several_integration_hooks_still_dispatch_once_per_record() {
    // The named subsystems share one entry point, so dispatching per hook
    // would count a single stage outcome three times.
    let plan = plan_dispatch(&[record(
        "a",
        Verification::Accepted,
        &["sona", "reasoning_bank", "desc"],
    )]);
    assert_eq!(plan.calls.len(), 1);
}

#[test]
fn unfinished_stages_are_not_attributed() {
    let plan = plan_dispatch(&[
        record("a", Verification::Unverified, &["sona"]),
        record("b", Verification::Failed, &["sona"]),
    ]);
    // A failure is an outcome worth learning from; a stage that never reached
    // a terminal state is not.
    assert_eq!(plan.calls.len(), 1);
    assert_eq!(plan.calls[0].task, "demo / b");
    assert_eq!(plan.skipped_incomplete, 1);
}

#[test]
fn quality_falls_back_to_verification_strength() {
    let accepted = plan_dispatch(&[record("a", Verification::Accepted, &["sona"])]);
    let forced = plan_dispatch(&[record("a", Verification::Forced, &["sona"])]);
    let failed = plan_dispatch(&[record("a", Verification::Failed, &["sona"])]);
    assert_eq!(accepted.calls[0].quality, 1.0);
    assert!(forced.calls[0].quality > 0.0 && forced.calls[0].quality < 1.0);
    assert_eq!(failed.calls[0].quality, 0.0);
}

#[test]
fn a_run_with_no_records_is_not_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let store = WorkflowStore::new(root.join("workflows"));
    let outcome = fold_workflow_learning(&root, &store, "no-such-run");
    assert_eq!(outcome.records_read, 0);
    assert_eq!(outcome.dispatched, 0);
    assert!(!outcome.integration_unavailable);
}

/// The bridge end to end: records on disk become episodes in the learning
/// store. Without this the two halves could each pass their own tests while
/// nothing crossed between them, which is exactly the state L3 exists to fix.
#[test]
fn hooked_records_reach_the_learning_store() {
    let _guard = super::store_lock();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::create_dir_all(root.join(".archon")).unwrap();
    let store = WorkflowStore::new(root.join("workflows"));

    let path = archon_workflow::learning_records_path(&store, "run-1");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let lines: String = [
        record("a", Verification::Accepted, &["sona"]),
        record("b", Verification::Accepted, &["world_model"]),
        record("c", Verification::Unverified, &["sona"]),
    ]
    .iter()
    .map(|record| format!("{}\n", serde_json::to_string(record).unwrap()))
    .collect();
    std::fs::write(&path, lines).unwrap();

    let outcome = fold_workflow_learning(&root, &store, "run-1");
    assert_eq!(outcome.records_read, 3);
    assert_eq!(outcome.dispatched, 1);
    assert_eq!(outcome.skipped_unhooked, 1);
    assert_eq!(outcome.skipped_incomplete, 1);
    assert!(!outcome.integration_unavailable);

    let db = super::open_db(&root.join(".archon").join("learning-state.db"));
    let episodes = db
        .run_script(
            "?[episode_id, description] := *desc_episodes{episode_id, description}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(episodes.rows.len(), 1);
    assert!(
        episodes.rows[0][1].get_str().unwrap().contains("demo / a"),
        "{:?}",
        episodes.rows[0]
    );
}

//! Coordination trace-row tests (#184 M9).

use super::*;

fn ok(text: &str) -> Result<String, String> {
    Ok(text.to_string())
}

#[test]
fn a_clean_merge_and_a_discard_are_different_outcomes() {
    assert_eq!(
        MergeOutcome::classify("merge", &ok("merged")),
        Some(MergeOutcome::Merged)
    );
    assert_eq!(
        MergeOutcome::classify("discard", &ok("discarded")),
        Some(MergeOutcome::Discarded)
    );
}

/// The label the whole path exists for. `exit_worktree` reports a conflict as
/// an error, so a conflict has to be read out of the failure rather than the
/// success.
#[test]
fn a_conflicting_merge_is_recognised_as_a_conflict() {
    let refused = Err("Merge has conflicts — manual resolution required".to_string());
    assert_eq!(
        MergeOutcome::classify("merge", &refused),
        Some(MergeOutcome::Conflicted)
    );
}

/// A merge that failed because the repository would not open says nothing about
/// whether the agents' work overlapped. Labelling it `merge_conflict` would
/// train the model on the filesystem.
#[test]
fn a_failure_that_is_not_a_conflict_records_nothing() {
    let broken = Err("cannot open the base repository: not a git repository".to_string());
    assert_eq!(MergeOutcome::classify("merge", &broken), None);
}

/// `keep` leaves the branch alone. Nothing was decided, so there is no outcome
/// to learn from.
#[test]
fn keeping_a_worktree_is_not_an_outcome() {
    assert_eq!(MergeOutcome::classify("keep", &ok("kept")), None);
}

#[test]
fn a_recorded_merge_carries_its_spawn_facts() {
    let dir = tempfile::tempdir().expect("temp dir");
    archon_tools::coordination_record::record_spawn(
        "m9-owner-1",
        SpawnFacts {
            label: Some("coder".into()),
            declared: vec!["src/lib.rs".into(), "src/main.rs".into()],
            claim_overlap: true,
            isolated: true,
            coordination_run_id: Some("team-9".into()),
        },
    );

    record_merge_outcome(
        dir.path(),
        "session-9",
        "m9-owner-1",
        MergeOutcome::Conflicted,
        4,
    );

    let store = WorldModelStore::open(dir.path()).expect("store");
    let rows = store.load_rows().expect("rows");
    let row = rows
        .iter()
        .find(|r| r.row_id == "world-row-merge-m9-owner-1")
        .expect("the merge row");

    assert_eq!(row.action_kind, WorldActionKind::WorktreeMerge);
    assert!(row.labels.merge_conflict);
    assert!(row.labels.claim_overlap);
    assert!(row.labels.isolated);
    assert_eq!(row.labels.success, Some(false));
    assert_eq!(row.coordination_run_id.as_deref(), Some("team-9"));
    assert_eq!(row.scalar_features.attempt_index, Some(4));

    // Consumed — nothing will read this agent's facts again.
    assert!(archon_tools::coordination_record::peek("m9-owner-1").is_none());
}

/// An agent with no recorded facts still produces a row. The merge result is
/// the ground truth and is worth keeping on its own; the missing context reads
/// as absent rather than as false.
#[test]
fn a_merge_without_spawn_facts_still_records_the_outcome() {
    let dir = tempfile::tempdir().expect("temp dir");

    record_merge_outcome(
        dir.path(),
        "session-9",
        "m9-owner-2",
        MergeOutcome::Merged,
        2,
    );

    let store = WorldModelStore::open(dir.path()).expect("store");
    let row = store
        .load_rows()
        .expect("rows")
        .into_iter()
        .find(|r| r.row_id == "world-row-merge-m9-owner-2")
        .expect("the merge row");

    assert!(!row.labels.merge_conflict);
    assert_eq!(row.labels.success, Some(true));
    assert!(row.coordination_run_id.is_none());
}

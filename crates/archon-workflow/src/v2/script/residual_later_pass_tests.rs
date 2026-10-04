//! Batch O2 (M1): what a later pass counts as new work.

use std::collections::BTreeSet;

use super::super::{Residual, ResidualSeverity};
use super::is_new;

fn gap(id: &str, description: &str) -> Residual {
    Residual {
        recorded_by: "verify-1".into(),
        id: id.into(),
        severity: ResidualSeverity::High,
        description: description.into(),
        files: Vec::new(),
        unit_tasks: BTreeSet::new(),
        recorded_summary: String::new(),
        host_built: false,
    }
}

#[test]
fn a_reworded_gap_is_no_new_work_and_a_different_one_is() {
    let owners = BTreeSet::from(["TASK-A".to_string()]);
    let original = gap(
        "gap-drift",
        "src/store.rs:12 drifts from the lane's recorded version on every write (wording 1)",
    );
    let carried = vec![(&owners, &original)];
    // The same id in new words: the same gap, never planned as new.
    assert!(!is_new(
        &gap("gap-drift", "the store lane drifts (wording 2)"),
        &carried
    ));
    // A new id with the same opening words: the same gap too.
    assert!(!is_new(
        &gap(
            "gap-drift-again",
            "src/store.rs:12 drifts from the lane's recorded version on every write (wording 3)"
        ),
        &carried
    ));
    // Another gap altogether is new work.
    assert!(is_new(
        &gap("gap-other", "src/lib.rs:4 drops a field"),
        &carried
    ));
}

#[test]
fn a_residual_plateau_carries_a_resumable_pause_cause() {
    let previous = super::super::ResidualPlan {
        rounds: vec![],
        reported: vec![(gap("F", "still fails"), "unresolved".into())],
    };
    // A planned gap that remains open after its judge ran.
    let mut previous = previous;
    previous.rounds.push(super::super::PlannedRound {
        key: "unit".into(),
        tasks: BTreeSet::from(["T".into()]),
        files: BTreeSet::new(),
        residuals: vec![gap("F", "still fails")],
        kind: super::super::RoundKind::Owned,
        unit_key: None,
        refusal: None,
        pass: 3,
    });
    let plan = super::stalled(4, &previous, &[], &BTreeSet::from(["F".into()]));
    assert!(
        plan.reported
            .iter()
            .any(|(_, why)| why.starts_with("no_progress:")),
        "a stall needs a pause cause, not final blockers: {:?}",
        plan.reported
    );
}

//! Issue-118: the final gate over an attempted second pass on resume.

use super::gate_tests::slot;
use super::second_pass_tests::*;
use super::tests::*;
use super::*;
use crate::v2::WorkflowV2Status;

#[test]
fn a_resume_that_skips_an_attempted_second_pass_round_still_weighs_its_verifier() {
    let w = package_world();
    let (recorded, first) = first_round(&w, "medium");
    let fix1 = round_call(&first, "review-remediate-residual-3", "remediate", 1);
    w.save(&record(
        fix1.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &[],
    ));
    std::thread::sleep(std::time::Duration::from_millis(5));
    let verify1 = round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[],
        &[(
            "gap-new",
            "high",
            "crates/b/src/lib.rs drops the lane's version",
        )],
    );
    w.save(&verify1);
    let round = second(&w).rounds[0].clone();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let fix2 = round_call(&round, "review-remediate-residual-5", "remediate", 2);
    w.save(&record(fix2, WorkflowV2Status::Accepted, &["TASK-B"], &[]));
    std::thread::sleep(std::time::Duration::from_millis(5));
    // Its verifier accepted but recorded the gap it was asked to fix again.
    w.save(&round_verdict(
        &round,
        "verification-wave-review-verify-residual-6",
        2,
        &[],
        &[(
            "gap-new",
            "high",
            "crates/b/src/lib.rs drops the lane's version",
        )],
    ));
    // A later session: both rounds were attempted, so neither is asked; the
    // executed plan holds only the replayed calls and the two slots.
    let calls = vec![recorded.call.clone(), slot(), second_slot()];
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        gate.blocking
            .iter()
            .any(|b| b.contains("gap-new") && b.contains("again")),
        "{gate:#?}"
    );
}

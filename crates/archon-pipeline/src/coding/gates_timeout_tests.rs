use super::super::compilation_gate::{ChildReap, TreeTermination};
use super::{CleanupOutcome, MAX_TIMEOUT_EVIDENCE_BYTES, compilation_timeout_result};
use std::time::Duration;

#[test]
fn compilation_timeout_evidence_is_bounded() {
    let result = compilation_timeout_result(
        &"x".repeat(2_000),
        Duration::from_millis(100),
        CleanupOutcome::TerminationRequestAccepted {
            reap: ChildReap::Succeeded,
        },
        TreeTermination::Confirmed,
    );

    assert!(result.evidence.len() <= MAX_TIMEOUT_EVIDENCE_BYTES);
    assert!(result.evidence.contains("...` exceeded limit"));
}
#[test]
fn compilation_timeout_surfaces_reap_failure_without_output() {
    let result = compilation_timeout_result(
        "cargo build",
        Duration::from_millis(100),
        CleanupOutcome::TerminationRequestAccepted {
            reap: ChildReap::Failed,
        },
        TreeTermination::StillPresent,
    );

    assert!(!result.gate_passed);
    assert_eq!(result.failures[0].description, "Compilation timed out");
    assert_eq!(
        result.failures[0].details,
        "Compilation timeout: command `cargo build` exceeded limit 100ms; direct child termination request accepted; direct child reap failed; process group still had members after the termination bound."
    );
    assert_eq!(result.failures[0].details, result.evidence);
    assert!(!result.evidence.contains("STDOUT:"));
    assert!(!result.evidence.contains("STDERR:"));
}

// A fault the observation detects itself is recorded on the check's result
// with its typed cause, and judged by it; the text is only the message.
// Stored results without a cause still read back and are judged by text.
#![cfg(unix)]

#[path = "support/isolation_fixture.rs"]
mod support;
use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::acceptance_scratch::faults::{FaultCause, SIBLING_ROOT_REFUSED, classify};
use support::*;

#[tokio::test]
async fn a_removed_git_metadata_and_a_refused_sibling_carry_their_typed_causes() {
    let (_t, result) = observe(&[
        ("test -f input", "repo_root"),
        ("rm -f .git && test -f input", "repo_root"),
        ("test -d ../project", "repo_root"),
    ])
    .await;
    let causes: Vec<_> = result
        .checks
        .iter()
        .map(|check| check.fault.clone())
        .collect();
    let removed = FaultCause::Removed {
        part: "source".into(),
    };
    assert_eq!(
        causes,
        vec![None, Some(removed), Some(FaultCause::SiblingRootRefused)],
        "{result:?}"
    );
    for check in &result.checks[1..] {
        // Judged by the cause alone: no text is consulted.
        let fault = classify(check.fault.as_ref(), "");
        assert!(matches!(fault, CheckFault::Check { .. }), "{check:?}");
    }
}

#[test]
fn a_stored_result_without_a_cause_reads_back_and_is_judged_by_its_text() {
    let error = format!("{SIBLING_ROOT_REFUSED}: ../project resolves to /work/project");
    let stored = serde_json::json!({"acceptance_id": "AC-X-001", "exit_code": null,
        "stdout": [], "stderr": [], "operational_error": error});
    let check: CheckResult = serde_json::from_value(stored).unwrap();
    assert_eq!(check.fault, None);
    let fault = classify(check.fault.as_ref(), &error);
    assert!(matches!(fault, CheckFault::Check { .. }), "{fault:?}");
    let written = serde_json::to_value(&check).unwrap();
    assert!(written.get("fault").is_none(), "{written}");
}

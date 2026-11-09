use super::*;
#[test]
fn skeleton_retry_distinguishes_call_cap_from_run_budget() {
    let mut pair: archon_workflow::breaker_verdict::PairVerdict = serde_json::from_value(serde_json::json!({"task_id":"task","id":"id","verdict":"budget_exhausted","killed_by":{},"patch_keys":{}})).unwrap();
    assert!(!retry(&pair), "call cap is deterministic");
    pair.run_budget_exhausted = true;
    assert!(retry(&pair));
    let resume = FreezeResume::none();
    resume.progress.saved(false);
    let report = FreezeIncomplete::pending("proof work remains", &resume.progress).report();
    assert!(report.contains("proof work remains"));
    assert!(!report.contains("made no progress"));
    pair.run_budget_exhausted = false;
    pair.verdict = Verdict::HostUnrun;
    assert!(
        !retry(&pair),
        "deterministic host failure is not a run budget"
    );
}

#[test]
fn classification_retries_only_typed_transient_faults() {
    use archon_workflow::v2::requirement_classes::ClassificationError;
    assert!(
        classification_ready(Some(&ClassificationError::from("repository_record_absent"))).is_ok()
    );
    let error = classification_ready(Some(&ClassificationError::Transient {
        transient: "provider fault".into(),
    }))
    .unwrap_err();
    assert_eq!(
        crate::command::workflow_host_fault::classify(&error),
        crate::command::workflow_host_fault::Class::Transient
    );
    assert_eq!(error.to_string(), "provider fault");
    let mut pair: archon_workflow::breaker_verdict::PairVerdict = serde_json::from_value(serde_json::json!({"task_id":"task","id":"id","verdict":"breaker_unavailable","killed_by":{},"patch_keys":{}})).unwrap();
    pair.attempts = vec![archon_workflow::breaker_verdict::AttemptRecord {
        attempt: 1,
        reply_key: "reply".into(),
        outcome: archon_workflow::breaker_verdict::AttemptOutcome::Unavailable {
            cause: UnavailableCause::Transient,
            reason: "provider fault".into(),
        },
        refusals: vec![],
    }];
    assert!(retry(&pair));
}

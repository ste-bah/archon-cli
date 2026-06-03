use archon_workflow::context::output_reports_blocked;

#[test]
fn successful_missing_evidence_note_is_not_blocked() {
    let body = "status: complete\nNo blocking missing evidence. Source files were empty by design.";
    assert_eq!(output_reports_blocked(body), None);
}

#[test]
fn explicit_blocked_status_is_blocked() {
    let body = "**Status:** blocked\nMissing required evidence.";
    assert!(output_reports_blocked(body).is_some());
}

#[test]
fn empty_findings_with_audit_impossible_is_blocked() {
    let body = "findings: []\nCannot audit because source evidence is missing.";
    assert!(output_reports_blocked(body).is_some());
}

#[test]
fn policy_discussion_words_do_not_fail_stage() {
    let body = "\
status: complete
Gap register:
- Missing evidence items are tracked for downstream work.
- The workflow blocks unsafe actions before execution.
- Reviewers should not sign off on unverified artifacts.
";
    assert_eq!(output_reports_blocked(body), None);
}

#[test]
fn explicit_reject_verdict_is_blocked() {
    assert!(output_reports_blocked("Verdict: reject").is_some());
    assert!(output_reports_blocked("verdict: REJECT — DO NOT SIGN OFF").is_some());
    assert!(output_reports_blocked(r#"{"verdict":"reject"}"#).is_some());
}

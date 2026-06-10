use archon_workflow::context::{output_reports_blocked, output_reports_failed_verification};

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
fn markdown_status_heading_with_blocked_value_is_blocked() {
    let body = "### Status\n`blocked`\n\nNo command-execution tool was available.";
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

#[test]
fn structured_failed_verification_status_is_blocked() {
    let body = r#"
{
  "unit_id": "VU-TASK-TRL-002",
  "status": "failed",
  "summary": "Focused tests pass, but adversarial inspection found a real gap."
}
"#;
    assert_eq!(output_reports_blocked(body), None);
    assert!(output_reports_failed_verification(body).is_some());
}

#[test]
fn spaced_or_quoted_status_fields_are_blocked() {
    assert!(output_reports_failed_verification(r#"{ "status" : "failed" }"#).is_some());
    assert!(output_reports_failed_verification(r#"- "result" : "unverifiable","#).is_some());
}

#[test]
fn failed_timeout_status_is_blocked() {
    assert!(output_reports_failed_verification(r#"{"status":"failed_timeout"}"#).is_some());
    assert!(output_reports_failed_verification("status: failed_validation_timeout").is_some());
    assert!(output_reports_failed_verification("status: completed_with_timeouts").is_some());
    assert!(output_reports_failed_verification(r#"{ "status": "timed_out" }"#).is_some());
}

#[test]
fn unverifiable_verification_status_is_blocked() {
    assert!(output_reports_failed_verification("verification_status: unverifiable").is_some());
    assert!(
        output_reports_failed_verification(
            r#"{"overall_result":"partial_pass_with_timeout_residual"}"#
        )
        .is_some()
    );
}

#[test]
fn markdown_verification_heading_with_failed_value_is_blocked() {
    let body = "### Verification Status\n`unverifiable`\n\nNo fresh test evidence exists.";
    assert!(output_reports_failed_verification(body).is_some());
}

#[test]
fn ordinary_test_counts_do_not_block() {
    let body = "\
Focused tests completed.
test result: ok. 6 passed; 0 failed; 87 filtered out.
Failed inputs: 0
    No Failed inputs.
    ";
    assert_eq!(output_reports_blocked(body), None);
    assert_eq!(output_reports_failed_verification(body), None);
}

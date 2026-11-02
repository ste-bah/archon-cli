use super::*;
use archon_workflow::acceptance_scratch::{CHECK_TIMED_OUT, CheckResult};

fn result(id: &str, stdout: &[u8], operational_error: Option<&str>) -> CheckResult {
    CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        classification: None,
        acceptance_id: id.into(),
        exit_code: if operational_error.is_some() {
            None
        } else {
            Some(0)
        },
        quota_walk_count: 0,
        stdout: stdout.to_vec(),
        stderr: Vec::new(),
        operational_error: operational_error.map(str::to_string),
        environment_note: None,
    }
}

#[test]
fn a_check_timeout_is_not_classified_as_an_author_finding() {
    let set = frozen_set(&[("AC-X-001", "check-with-timeout", true)]);
    let finding = super::super::crash_findings(
        &set.contract(),
        [&result("AC-X-001", b"", Some(CHECK_TIMED_OUT))],
    );
    assert!(
        finding.is_empty(),
        "a timeout has no implementation verdict: {finding:?}"
    );
}

#[test]
fn a_zero_work_check_output_is_an_author_finding() {
    let set = frozen_set(&[("AC-F-001", "cargo test missing_test_name", true)]);
    let finding = super::super::crash_findings(
        &set.contract(),
        [&result(
            "AC-F-001",
            b"running 0 tests\ntest result: ok. 0 passed; 0 failed\n",
            None,
        )],
    );
    assert!(finding["AC-F-001"].contains("zero work"));
    assert!(finding["AC-F-001"].contains("running 0 tests"));
}

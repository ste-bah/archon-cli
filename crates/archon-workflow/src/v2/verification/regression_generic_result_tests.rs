use super::regression_compare::{RegressionFinding, compare};
use crate::v2::write::test_baseline_parse::{
    failing_tests, ignored_tests, passed_count, passed_tests,
};
use crate::v2::write::test_baseline_run_base::HostRunVerdict;
use std::collections::BTreeMap;

#[test]
fn non_native_deleted_and_skipped_tests_block_regression_comparison() {
    for (before, deleted, skipped) in [
        (
            "tests/test_a.py::test_one PASSED\ntests/test_a.py::test_two PASSED",
            "tests/test_a.py::test_one PASSED",
            "tests/test_a.py::test_one PASSED\ntests/test_a.py::test_two SKIPPED",
        ),
        (
            "--- PASS: TestOne (0.00s)\n--- PASS: TestTwo (0.00s)",
            "--- PASS: TestOne (0.00s)",
            "--- PASS: TestOne (0.00s)\n--- SKIP: TestTwo (0.00s)",
        ),
        (
            "ok 1 - first\nok 2 - second",
            "ok 1 - first",
            "ok 1 - first\nok 2 - second # SKIP reason",
        ),
        (
            "2 passed in 0.1s",
            "1 passed in 0.1s",
            "1 passed, 1 skipped in 0.1s",
        ),
        (
            r#"{"test_id":"suite::one","status":"passed"}
{"test_id":"suite::two","status":"passed"}"#,
            r#"{"test_id":"suite::one","status":"passed"}"#,
            r#"{"test_id":"suite::one","status":"passed"}
{"test_id":"suite::two","status":"skipped"}"#,
        ),
    ] {
        let command = "declared-runner".to_string();
        let verdict = |output: &str| HostRunVerdict {
            command: command.clone(),
            exit_code: Some(0),
            ids_kept: true,
            passed_count: passed_count(output),
            failing_tests: failing_tests(output),
            passed_tests: passed_tests(output),
            ignored_tests: ignored_tests(output),
            ..Default::default()
        };
        let base = BTreeMap::from([(command.clone(), verdict(before))]);
        for output in [deleted, skipped] {
            let tip = BTreeMap::from([(command.clone(), verdict(output))]);
            let findings = compare(&[command.clone()], &base, &tip);
            assert!(
                findings.iter().any(|finding| matches!(
                    finding,
                    RegressionFinding::Vanished { .. }
                        | RegressionFinding::Hidden { .. }
                        | RegressionFinding::PassingCountDecreased { .. }
                )),
                "{before} -> {output}: {findings:?}"
            );
            assert!(findings.iter().any(RegressionFinding::blocks));
        }
    }
}

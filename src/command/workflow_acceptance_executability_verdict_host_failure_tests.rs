use super::*;

#[test]
fn host_failure_reads_stored_classification_without_command_text() {
    let mut result = run(Some(0), "", "sh: line 1: X: unbound variable");
    result.classify_raw("set -u; echo $X");
    result.stderr.clear();
    assert!(!may_be_host_failure(&result));
    assert!(!result.passed());
    result.exit_code = Some(127);
    result.classification = None;
    assert!(may_be_host_failure(&result));
}

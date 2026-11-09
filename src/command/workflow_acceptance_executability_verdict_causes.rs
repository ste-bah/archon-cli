//! Separate the host availability rules from the compile-location rule.
use super::*;
pub(crate) fn no_verdict_cause(
    command: &str,
    result: &CheckResult,
    at: &Context,
) -> Option<(String, UnprovenCause)> {
    no_verdict_cause_inner(command, result, at, true)
}
pub(crate) fn mutant_cause(
    command: &str,
    result: &CheckResult,
    at: &Context,
) -> Option<(String, UnprovenCause)> {
    no_verdict_cause_inner(command, result, at, false)
}
pub(crate) fn output(result: &CheckResult) -> String {
    format!(
        "{}\n{}",
        // Issue 544: verdicts read the raw capture when it is kept.
        String::from_utf8_lossy(result.verdict_stdout()),
        String::from_utf8_lossy(result.verdict_stderr())
    )
}

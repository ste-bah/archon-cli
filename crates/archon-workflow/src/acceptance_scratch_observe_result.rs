//! Constructors and redaction for per-check observation results.

use super::*;

pub(super) fn redacted(roots: &ScratchRoots, check: &CheckResult) -> CheckResult {
    let mut check = check.clone();
    check.stdout = roots.redact(&check.stdout);
    check.stderr = roots.redact(&check.stderr);
    if let Some(error) = &mut check.operational_error {
        *error = String::from_utf8_lossy(&roots.redact(error.as_bytes())).into_owned();
    }
    check
}

pub(super) fn operational(id: &str, error: String) -> CheckResult {
    CheckResult {
        classification: None,
        acceptance_id: id.into(),
        exit_code: None,
        quota_walk_count: 0,
        stdout: vec![],
        stderr: vec![],
        operational_error: Some(error),
        fault: None,
        environment_note: None,
    }
}

/// [`operational`], recording the fault's typed `cause` when it has one.
pub(super) fn faulted(id: &str, error: String, cause: Option<faults::FaultCause>) -> CheckResult {
    CheckResult {
        fault: cause,
        ..operational(id, error)
    }
}

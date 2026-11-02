//! Constructors and redaction for per-check observation results.

use super::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct ObservationResult {
    pub checks: Vec<CheckResult>,
    pub host_environment: BTreeMap<String, bool>,
    pub check_evidence: Vec<CheckEvidence>,
    pub operational_errors: Vec<String>,
    pub command_refs: Vec<FrozenCommandRef>,
    pub command_cwds: Vec<crate::task_set_contract::TrustedCwd>,
    pub policy: ScratchPolicy,
    pub copied_project_manifest: BTreeMap<String, String>,
    pub cleanup_error: Option<String>,
    /// The check whose changes left a scratch site the host could not remove.
    /// Its finding stays with that check and does not void the observation.
    #[serde(default)]
    pub attributed_teardown: Option<AttributedTeardown>,
    pub live_roots_unchanged: bool,
    /// True when cleanup succeeded or its failure was attributed to a check.
    /// An attributed site may remain on disk; false means an unattributed
    /// teardown failure remains.
    pub teardown_verified: bool,
    pub source_commit: String,
    pub policy_digest: String,
    pub source_manifest_digest: String,
    pub before: BTreeMap<String, BTreeMap<String, String>>,
    pub after: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AttributedTeardown {
    pub acceptance_id: String,
    pub error: String,
}

impl ObservationResult {
    pub fn passed(&self) -> bool {
        self.operational_errors.is_empty()
            && self.live_roots_unchanged
            && self.teardown_verified
            && !self.checks.is_empty()
            && self
                .checks
                .iter()
                .all(|c| c.exit_code == Some(0) && c.operational_error.is_none())
    }
}

pub(super) fn redacted(roots: &ScratchRoots, check: &CheckResult) -> CheckResult {
    redacted_environment(&roots.redaction_environment, check)
}

pub(super) fn redacted_environment(
    host_environment: &BTreeMap<String, String>,
    check: &CheckResult,
) -> CheckResult {
    let mut check = check.clone();
    check.stdout = super::io::redact(host_environment, &check.stdout, false);
    check.stderr = super::io::redact(host_environment, &check.stderr, false);
    if let Some(error) = &mut check.operational_error {
        *error = String::from_utf8_lossy(&super::io::redact(
            host_environment,
            error.as_bytes(),
            false,
        ))
        .into_owned();
    }
    check
}

pub(super) fn operational(id: &str, error: String) -> CheckResult {
    CheckResult {
        verdict_streams: Default::default(),
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

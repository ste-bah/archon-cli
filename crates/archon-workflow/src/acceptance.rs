//! Phase B implementation-stage acceptance binding.
//!
//! An `implementation` stage is accepted ONLY when both conditions hold:
//!   1. every declared `expected_target_files` entry changed on disk during the
//!      stage (content fingerprint differs, including create/delete), AND
//!   2. the stage `verify_command` (when present) exits with status 0.
//!
//! This is the structural guard that makes a write-capable stage trustworthy:
//! a stage that claims success without mutating its targets — or whose
//! verification fails — is rejected rather than silently accepted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A fingerprint of a single target path. `None` means the path is absent.
pub type TargetFingerprints = BTreeMap<String, Option<String>>;

/// Outcome of evaluating implementation-stage acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptanceOutcome {
    Accepted,
    Rejected(String),
}

impl AcceptanceOutcome {
    pub fn is_accepted(&self) -> bool {
        matches!(self, AcceptanceOutcome::Accepted)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            AcceptanceOutcome::Accepted => None,
            AcceptanceOutcome::Rejected(reason) => Some(reason.as_str()),
        }
    }
}

/// Fingerprint each declared target relative to `root` (or absolute).
pub fn snapshot_targets(root: &Path, targets: &[String]) -> TargetFingerprints {
    targets
        .iter()
        .map(|target| (target.clone(), fingerprint(root, target)))
        .collect()
}

/// Targets are considered mutated when every entry's fingerprint differs
/// between `before` and `after`. A target whose fingerprint is unchanged
/// (including one that stayed absent) fails the mutation requirement.
pub fn unmutated_targets(before: &TargetFingerprints, after: &TargetFingerprints) -> Vec<String> {
    before
        .keys()
        .filter(|target| before.get(*target) == after.get(*target))
        .cloned()
        .collect()
}

/// Run the stage verification command in `root`. Returns `Ok(())` on exit 0,
/// otherwise an error describing the failure. `None` command always passes.
pub fn run_verify_command(root: &Path, command: Option<&str>) -> Result<(), String> {
    let Some(command) = command else {
        return Ok(());
    };
    let command = command.trim();
    if command.is_empty() {
        return Ok(());
    }
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(root)
        .output()
        .map_err(|err| format!("verify_command failed to launch: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let code = output
        .status
        .code()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal".to_string());
    Err(format!("verify_command exited with status {code}"))
}

/// Combine the mutation check and verification into a single acceptance verdict.
pub fn evaluate(
    root: &Path,
    targets: &[String],
    before: &TargetFingerprints,
    after: &TargetFingerprints,
    verify_command: Option<&str>,
) -> AcceptanceOutcome {
    let unmutated = unmutated_targets(before, after);
    if !unmutated.is_empty() {
        return AcceptanceOutcome::Rejected(format!(
            "expected_target_files not modified: {}",
            unmutated.join(", ")
        ));
    }
    if targets.is_empty() {
        return AcceptanceOutcome::Rejected(
            "implementation stage declared no expected_target_files".to_string(),
        );
    }
    match run_verify_command(root, verify_command) {
        Ok(()) => AcceptanceOutcome::Accepted,
        Err(reason) => AcceptanceOutcome::Rejected(reason),
    }
}

fn fingerprint(root: &Path, target: &str) -> Option<String> {
    let path = resolve(root, target);
    let bytes = std::fs::read(path).ok()?;
    Some(blake3::hash(&bytes).to_hex().to_string())
}

fn resolve(root: &Path, target: &str) -> PathBuf {
    let raw = Path::new(target);
    if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmutated_targets_detects_unchanged_and_missing() {
        let mut before = TargetFingerprints::new();
        before.insert("a".into(), Some("h1".into()));
        before.insert("b".into(), None);
        let mut after = TargetFingerprints::new();
        after.insert("a".into(), Some("h1".into())); // unchanged
        after.insert("b".into(), Some("h2".into())); // created
        let unmutated = unmutated_targets(&before, &after);
        assert_eq!(unmutated, vec!["a".to_string()]);
    }

    #[test]
    fn verify_command_none_and_empty_pass() {
        let root = std::env::temp_dir();
        assert!(run_verify_command(&root, None).is_ok());
        assert!(run_verify_command(&root, Some("   ")).is_ok());
    }

    #[test]
    fn verify_command_failure_is_reported() {
        let root = std::env::temp_dir();
        let err = run_verify_command(&root, Some("exit 3")).unwrap_err();
        assert!(err.contains('3'), "reason should carry exit code: {err}");
    }

    #[test]
    fn evaluate_rejects_when_no_targets_declared() {
        let root = std::env::temp_dir();
        let before = TargetFingerprints::new();
        let after = TargetFingerprints::new();
        let outcome = evaluate(&root, &[], &before, &after, None);
        assert!(!outcome.is_accepted());
    }
}

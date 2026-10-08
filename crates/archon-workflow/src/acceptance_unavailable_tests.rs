use super::*;
use crate::acceptance_check_environment::CheckPolicy;

fn evaluate(root: &Path, policy: &CheckPolicy) -> AcceptanceOutcome {
    let target = "output.txt";
    std::fs::write(root.join(target), "present").unwrap();
    evaluate_with_policy(
        root,
        &[target.to_string()],
        &TargetFingerprints::new(),
        &snapshot_targets(root, &[target.to_string()]),
        Some("true"),
        Some(policy),
    )
}

#[test]
fn missing_allowlisted_variable_is_unavailable_not_rejected() {
    let root = tempfile::tempdir().unwrap();
    let policy = CheckPolicy {
        toolchain_path: Some("/usr/bin:/bin".into()),
        forwarded: vec!["ARCHON_349_ABSENT_DATA".into()],
        ..Default::default()
    };
    assert!(matches!(
        evaluate(root.path(), &policy),
        AcceptanceOutcome::Unavailable(_)
    ));
}

#[test]
fn invalid_environment_binding_is_unavailable_not_rejected() {
    let root = tempfile::tempdir().unwrap();
    let policy = CheckPolicy {
        toolchain_path: Some("/usr/bin:/bin".into()),
        bound: std::collections::BTreeMap::from([("LANG".into(), "bad\0value".into())]),
        ..Default::default()
    };
    assert!(matches!(
        evaluate(root.path(), &policy),
        AcceptanceOutcome::Unavailable(_)
    ));
}

// Unix only: on Windows `shell_program()` can name an absolute `sh.exe`
// beside `git`, so a missing toolchain PATH does not stop the launch.
#[cfg(unix)]
#[test]
fn verifier_launch_failure_is_unavailable_not_rejected() {
    let root = tempfile::tempdir().unwrap();
    let policy = CheckPolicy {
        toolchain_path: Some("/definitely/missing".into()),
        ..Default::default()
    };
    assert!(matches!(
        evaluate(root.path(), &policy),
        AcceptanceOutcome::Unavailable(_)
    ));
}

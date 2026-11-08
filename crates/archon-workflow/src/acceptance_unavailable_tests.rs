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

#[test]
fn verifier_launch_failure_is_unavailable_not_rejected() {
    let root = tempfile::tempdir().unwrap();
    let policy = CheckPolicy {
        toolchain_path: Some("/definitely/missing".into()),
        ..Default::default()
    };
    let targets = vec!["output.txt".to_string()];
    std::fs::write(root.path().join(&targets[0]), "present").unwrap();
    let after = snapshot_targets(root.path(), &targets);
    // An absent working directory forces an actual spawn failure even when
    // the host shell is pinned independently of the child's PATH.
    let outcome = evaluate_with_policy(
        &root.path().join("absent"),
        &targets,
        &TargetFingerprints::new(),
        &after,
        Some("true"),
        Some(&policy),
    );
    let AcceptanceOutcome::Unavailable(reason) = outcome else {
        panic!("launch failure must be unavailable: {outcome:?}");
    };
    assert!(reason.contains("failed to launch"), "{reason}");
}

#[test]
fn pinned_host_shell_survives_a_missing_child_path() {
    let root = tempfile::tempdir().unwrap();
    let policy = CheckPolicy {
        toolchain_path: Some("/definitely/missing".into()),
        ..Default::default()
    };
    assert!(matches!(
        evaluate(root.path(), &policy),
        AcceptanceOutcome::Accepted
    ));
}

use super::*;

#[test]
fn a_retry_is_granted_every_file_the_hosts_judged_run_saw_its_test_fail_in() {
    use crate::v2::write::test_baseline_run_base::{HostRunVerdict, Tree, cache};
    let w = package_world();
    let extra = "crates/shared/src/fixtures.rs";
    std::fs::write(w.root().join(extra), "//\n").unwrap();
    for args in [
        &["init", "-q"][..],
        &["config", "user.name", "t"],
        &["config", "user.email", "t@example.invalid"],
        &["add", "."],
        &["commit", "-qm", "fixture"],
    ] {
        crate::write_coordinator::worktree_isolation::run_git(args, w.root()).unwrap();
    }
    let commit = String::from_utf8(
        crate::write_coordinator::worktree_isolation::run_git(&["rev-parse", "HEAD"], w.root())
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let (_, first) = first_round(&w, "high");
    let mut judge = round_verdict(
        &w,
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[STALE],
        &[],
    );
    judge.result.data["outcomes"][0]["result"]["data"]["judged_commit"] = json!(commit);
    w.save(&judge);
    cache(
        &w.store,
        Tree::Judged,
        &HostRunVerdict {
            verbatim: true,
            isolated: true,
            per_test_verdict: true,
            command: LIB.into(),
            commit: commit.clone(),
            exit_code: Some(101),
            failing_tests: vec![STALE.into()],
            failure_files: [(
                STALE.to_string(),
                vec![TESTS.to_string(), extra.to_string()],
            )]
            .into(),
            ..Default::default()
        },
    );
    let retry = second(&w).rounds[0].clone();
    assert_eq!(ids(&retry.files), [extra, STORE, TESTS]);
}

#[test]
fn an_unlocated_runner_id_remains_an_unresolved_red() {
    let w = package_world();
    let (_, first) = first_round(&w, "high");
    w.save(&round_verdict(
        &w,
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &["store::tests::unlocated"],
        &[],
    ));
    let retry = second(&w);
    assert!(!retry.rounds.is_empty());
    assert!(
        retry.rounds[0]
            .residuals
            .iter()
            .any(|residual| residual.description.contains("store::tests::unlocated"))
    );
}

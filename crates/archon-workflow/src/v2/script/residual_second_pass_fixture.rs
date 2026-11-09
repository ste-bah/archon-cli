use super::*;

/// The Issue-117 world plus a package the host can resolve test ids in.
pub(in crate::v2::script::residual_plan) fn package_world() -> World {
    let mut w = world();
    w.universe.tasks[0].focused_tests = vec![LIB.into(), "cargo test -p b --test registry".into()];
    w.universe.tasks[0].focused_commands = Some(w.universe.tasks[0].focused_tests.clone());
    std::fs::write(
        w.root().join("crates/shared/Cargo.toml"),
        "[package]\nname = \"shared\"\n",
    )
    .unwrap();
    std::fs::write(w.root().join(TESTS), "//\n").unwrap();
    for args in [
        &["init", "-q"][..],
        &["config", "user.name", "t"],
        &["config", "user.email", "t@example.invalid"],
        &["add", "."],
        &["commit", "-qm", "fixture"],
    ] {
        crate::write_coordinator::worktree_isolation::run_git(args, w.root()).unwrap();
    }
    let commit = crate::repository_record::git_head(w.root()).unwrap();
    crate::v2::write::test_baseline_run_base::cache(
        &w.store,
        crate::v2::write::test_baseline_run_base::Tree::Judged,
        &crate::v2::write::test_baseline_run_base::HostRunVerdict {
            command: LIB.into(),
            commit,
            verbatim: true,
            isolated: true,
            per_test_verdict: true,
            failing_tests: vec![STALE.into()],
            failure_files: [(STALE.into(), vec![TESTS.into()])].into(),
            ..Default::default()
        },
    );
    w
}

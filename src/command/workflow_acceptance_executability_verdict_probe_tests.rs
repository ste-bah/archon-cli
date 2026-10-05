//! Issue 328 through a real probe on real trees: a check that fails on the
//! pre-implementation tree only because that tree did not build, or a tool
//! it runs is missing, gave no verdict there -- unproven, never a proof --
//! while a genuine assertion failure there is still a proof.

use archon_workflow::task_set_contract::TrustedCwd;

use super::probe_tests::{git, trees};
use super::{Baseline, ExecutabilityProbe, HostProbe};

const REPO: TrustedCwd = TrustedCwd::RepoRoot;

/// Compiles the tree's library, then asserts the feature exists.
const BUILDS_THEN_ASSERTS: &str = "rustc --crate-type lib --emit=metadata --out-dir \"$(mktemp -d)\" src/lib.rs && test -f feature.txt";

/// The round's own site (live, no policy) held to `commit`.
fn round_at(
    trees: &super::probe_tests::Trees,
    commit: &str,
    copies: &std::path::Path,
) -> HostProbe {
    HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        None,
    )
    .with_baseline(Baseline {
        commit: commit.to_string(),
        repository: trees.repo.clone(),
    })
    .with_copy_parent(copies.to_path_buf())
}

/// Commit `files` (path, text) to `repo`; returns the new HEAD.
fn commit(repo: &std::path::Path, files: &[(&str, &str)], message: &str) -> String {
    for (path, text) in files {
        std::fs::write(repo.join(path), text).unwrap();
    }
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", message]);
    git(repo, &["rev-parse", "HEAD"])
}

#[tokio::test]
async fn a_check_failing_only_because_the_base_does_not_build_is_unproven_not_proven() {
    let trees = trees(&[("AC-328-001", BUILDS_THEN_ASSERTS, REPO)]);
    // The pre-implementation tree's own code does not compile; the live
    // tree's does, and has the feature.
    let broken = commit(
        &trees.repo,
        &[("src/lib.rs", "pub fn f() -> u32 { \"x\" }\n")],
        "broken base",
    );
    commit(
        &trees.repo,
        &[("src/lib.rs", "pub fn f() -> u32 { 1 }\n")],
        "fixed",
    );
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &broken, copies.path());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    assert!(findings.is_empty(), "never the author's: {findings:?}");
    let why = unproven.get("AC-328-001").unwrap_or_else(|| {
        panic!(
            "a compile error at base is no proof it can fail: {unproven:?} {:?}",
            probe.take_diagnostics()
        )
    });
    assert!(why.contains("did not build"), "{why}");
    assert!(why.contains(&broken[..12]), "{why}");
    assert!(why.contains("mismatched types"), "stderr evidence: {why}");
    let runs = probe.take_baseline_runs().expect("a baseline");
    assert!(runs.failures.is_empty(), "no verdict to judge: {runs:?}");
    trees.assert_live_untouched(copies.path());
}

#[tokio::test]
async fn a_check_failing_only_because_a_tool_is_missing_is_unproven_not_proven() {
    let missing = "archon-issue-328-absent-tool --verify feature.txt";
    let trees = trees(&[("AC-328-002", missing, REPO)]);
    let copies = tempfile::tempdir().unwrap();
    // A freeze: its site is the hermetic copy of the baseline itself.
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    assert!(findings.is_empty(), "never the author's: {findings:?}");
    let why = unproven
        .get("AC-328-002")
        .unwrap_or_else(|| panic!("a missing tool is no proof it can fail: {unproven:?}"));
    assert!(why.contains("could not be started (exit 127"), "{why}");
    trees.assert_live_untouched(copies.path());
}

#[tokio::test]
async fn a_genuine_assertion_failure_on_the_base_is_still_a_proof() {
    let trees = trees(&[
        ("AC-328-003", "test -f feature.txt", REPO),
        ("AC-328-004", BUILDS_THEN_ASSERTS, REPO),
    ]);
    // The base builds; only the assertion fails there.
    let base = commit(
        &trees.repo,
        &[("src/lib.rs", "pub fn f() -> u32 { 1 }\n")],
        "base builds",
    );
    git(&trees.repo, &["rm", "-q", "feature.txt"]);
    git(&trees.repo, &["commit", "-qm", "before the feature"]);
    let base_without = git(&trees.repo, &["rev-parse", "HEAD"]);
    commit(&trees.repo, &[("feature.txt", "built")], "feature");
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &base_without, copies.path());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.is_empty(), "{findings:?}");
    assert!(
        probe.take_unproven().is_empty(),
        "{:?}",
        probe.take_diagnostics()
    );
    let runs = probe.take_baseline_runs().expect("a baseline");
    let failed: Vec<&String> = runs.failures.keys().collect();
    assert_eq!(failed, vec!["AC-328-003", "AC-328-004"], "both proven");
    assert_ne!(base, base_without);
}

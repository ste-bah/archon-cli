//! Issue 420 (gap 2) through a real freeze probe on real trees: a check
//! whose pre-implementation failure is a setup path outside the probe's
//! copy (a sibling root the copy does not hold) is unproven with that
//! cause, never a baseline failure the passability judge is shown; a check
//! failing on an absent deliverable inside the tree is still a proof.

use super::probe_tests::trees;
use super::verdict_probe_tests::REPO;
use super::{ExecutabilityProbe, HostProbe};

#[tokio::test]
async fn a_sibling_root_absent_from_the_copy_is_unproven_and_never_judged() {
    let sibling = "cargo metadata --no-deps --format-version 1 --manifest-path ../archon-420-code-root/Cargo.toml";
    let enters = "cd ../archon-420-code-root && test -f feature.txt";
    let deliverable = "cargo metadata --no-deps --format-version 1 --manifest-path crates/archon-420-new/Cargo.toml";
    // `../repo` is the repository copy the probe's copy holds: a deliverable.
    let held_root = "cargo metadata --no-deps --format-version 1 --manifest-path ../repo/crates/archon-420-new/Cargo.toml";
    let trees = trees(&[
        ("AC-420-001", sibling, REPO),
        ("AC-420-002", enters, REPO),
        ("AC-420-003", deliverable, REPO),
        ("AC-420-004", held_root, REPO),
    ]);
    // The sibling root exists beside the live repository, so the live
    // layout would find it; the probe's copy holds only the repository and
    // the project.
    let live_sibling = trees.outside.path().join("archon-420-code-root");
    std::fs::create_dir_all(&live_sibling).unwrap();
    std::fs::write(live_sibling.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    assert!(
        findings.is_empty(),
        "never the author's on the first strike: {findings:?}"
    );
    for (id, path) in [
        ("AC-420-001", "../archon-420-code-root/Cargo.toml"),
        ("AC-420-002", "../archon-420-code-root"),
    ] {
        let why = unproven.get(id).unwrap_or_else(|| {
            panic!(
                "{id}: a setup failure counted as a proof: {unproven:?} {:?}",
                probe.take_diagnostics()
            )
        });
        assert!(why.contains("outside the tree"), "{id}: {why}");
        assert!(why.contains(path), "{id}: {why}");
        assert!(why.contains(&trees.base[..12]), "{id}: {why}");
    }
    assert!(!unproven.contains_key("AC-420-003"), "{unproven:?}");
    assert!(!unproven.contains_key("AC-420-004"), "{unproven:?}");
    let runs = probe.take_baseline_runs().expect("a baseline");
    let judged: Vec<&String> = runs.failures.keys().collect();
    assert_eq!(
        judged,
        vec!["AC-420-003", "AC-420-004"],
        "only the proofs reach passability"
    );
    trees.assert_live_untouched(copies.path());
}

/// The live tree (an acceptance round's own site, no scratch policy): a
/// project beside its repository runs `--manifest-path ../repo/...`, a real
/// path there. An absent crate is the deliverable not built yet, so the
/// original failed its criterion and is held (A5), never a defect.
#[tokio::test]
async fn on_the_live_tree_a_sibling_repository_path_stays_a_held_failure() {
    let command = "cargo metadata --no-deps --format-version 1 --manifest-path ../repo/crates/new-feature/Cargo.toml";
    let trees = trees(&[("AC-420-010", command, REPO)]);
    let project = trees.outside.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: "AC-420-010".into(),
        exit_code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        quota_walk_count: 0,
        operational_error: None,
        classification: None,
        environment_note: None,
    };
    let contract = trees.contract();
    let site = HostProbe::at(project, trees.repo.clone(), None).check_site(&contract);
    let originals = super::originals(&site, &contract, vec![result]).await;
    assert_eq!(originals.get("AC-420-010"), Some(&super::Original::Failed));
}

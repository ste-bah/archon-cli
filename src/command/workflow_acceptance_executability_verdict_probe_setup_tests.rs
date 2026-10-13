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
    let deliverable =
        "cargo metadata --no-deps --format-version 1 --manifest-path crates/archon-420-new/Cargo.toml";
    let trees = trees(&[
        ("AC-420-001", sibling, REPO),
        ("AC-420-002", enters, REPO),
        ("AC-420-003", deliverable, REPO),
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
    assert!(findings.is_empty(), "never the author's on the first strike: {findings:?}");
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
    let runs = probe.take_baseline_runs().expect("a baseline");
    let judged: Vec<&String> = runs.failures.keys().collect();
    assert_eq!(judged, vec!["AC-420-003"], "only the proof reaches passability");
    trees.assert_live_untouched(copies.path());
}

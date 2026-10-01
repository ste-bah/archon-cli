//! REM-16 (Major 2): the review tripwire over a real git checkout and over
//! roots elsewhere on disk that the run's records name.

use std::path::{Path, PathBuf};

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

use super::super::review_roots::review_roots;
use super::{ReviewTreeTripwire, WatchSet};

fn git(root: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git runs");
    assert!(status.status.success(), "git {args:?}: {status:?}");
}

/// A temp dir holding a committed checkout (`repo`) with an ignored build
/// directory, an ignored deliverable directory, and one file dirty before
/// the review (the tripwire must keep it as it was).
fn world() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("repo");
    std::fs::create_dir_all(root.join("out")).unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.invalid"]);
    std::fs::write(root.join("lib.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join("dirty.rs"), "committed\n").unwrap();
    std::fs::write(root.join(".gitignore"), "target/\nout/\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "base"]);
    std::fs::write(root.join("dirty.rs"), "dirty before the review\n").unwrap();
    std::fs::write(root.join("out/report.md"), "the deliverable\n").unwrap();
    (dir, root)
}

fn checkout_only(root: &Path) -> WatchSet {
    WatchSet {
        repo: Some(root.to_path_buf()),
        roots: vec![root.to_path_buf()],
        excludes: vec![root.join(".git")],
    }
}

fn arm(dir: &tempfile::TempDir, watch: WatchSet) -> ReviewTreeTripwire {
    ReviewTreeTripwire::arm(watch, &dir.path().join("run/spill")).expect("armed")
}

#[test]
fn a_branch_that_leaves_the_tree_alone_passes_and_build_output_is_not_the_tree() {
    let (dir, root) = world();
    let wire = arm(&dir, checkout_only(&root));
    wire.enter("a");
    std::fs::create_dir_all(root.join("target/debug")).unwrap();
    std::fs::write(root.join("target/debug/out"), "built").unwrap();
    // A touch moves the mtime but not the content: not a change.
    let file = std::fs::File::options()
        .write(true)
        .open(root.join("lib.rs"))
        .unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
        .unwrap();
    assert_eq!(wire.leave("a"), Ok(()));
}

#[test]
fn a_change_fails_the_branch_and_is_reverted() {
    let (dir, root) = world();
    let wire = arm(&dir, checkout_only(&root));
    wire.enter("a");
    std::fs::write(root.join("lib.rs"), "fn a() { broken }\n").unwrap();
    std::fs::write(root.join("new.rs"), "stray\n").unwrap();
    std::fs::write(root.join("dirty.rs"), "overwritten\n").unwrap();
    let error = wire.leave("a").expect_err("the branch fails");
    assert!(error.contains("lib.rs restored"), "{error}");
    let read = |name: &str| std::fs::read_to_string(root.join(name)).unwrap();
    assert_eq!(read("lib.rs"), "fn a() {}\n");
    assert!(!root.join("new.rs").exists(), "a new file is removed");
    assert_eq!(
        read("dirty.rs"),
        "dirty before the review\n",
        "a path dirty before gets its spilled bytes back"
    );
    wire.enter("b");
    assert_eq!(wire.leave("b"), Ok(()), "the tree is the baseline again");
}

/// An ignored file is part of the tree: deliverables live there.
#[test]
fn a_change_to_an_ignored_file_is_caught_and_restored() {
    let (dir, root) = world();
    let wire = arm(&dir, checkout_only(&root));
    wire.enter("a");
    std::fs::write(root.join("out/report.md"), "faked\n").unwrap();
    std::fs::write(root.join("out/extra.md"), "planted\n").unwrap();
    assert!(wire.leave("a").is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("out/report.md")).unwrap(),
        "the deliverable\n"
    );
    assert!(!root.join("out/extra.md").exists());
}

#[test]
fn every_branch_in_flight_when_the_change_is_found_fails() {
    let (dir, root) = world();
    let wire = arm(&dir, checkout_only(&root));
    wire.enter("innocent");
    wire.enter("culprit");
    std::fs::remove_file(root.join("lib.rs")).unwrap();
    assert!(wire.leave("innocent").is_err());
    assert!(root.join("lib.rs").exists(), "a deleted file is restored");
    assert!(
        wire.leave("culprit").is_err(),
        "never escapes by ending later"
    );
    wire.enter("later");
    assert_eq!(wire.leave("later"), Ok(()));
}

/// A root the task set names outside the checkout, under names nothing in
/// the host knows, is derived from the records and watched like the tree;
/// the spilled bytes go with the tripwire.
#[test]
fn a_declared_root_elsewhere_on_disk_is_watched_and_reverted() {
    let (dir, root) = world();
    let elsewhere = dir.path().join("elsewhere/xyz");
    std::fs::create_dir_all(elsewhere.join("deep")).unwrap();
    std::fs::write(elsewhere.join("deep/table.csv"), "a,b\n1,2\n").unwrap();
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-X".into(),
            artifact_requirements: vec![elsewhere.join("summary.json").display().to_string()],
            ..Default::default()
        }],
    };
    let run_root = dir.path().join("stores/run");
    std::fs::create_dir_all(&run_root).unwrap();
    let watch = review_roots(&run_root, None, &[], Some(&root), Some(&universe));
    assert!(watch.roots.contains(&elsewhere), "{watch:?}");
    let spill = dir.path().join("run/spill");
    let wire = ReviewTreeTripwire::arm(watch, &spill).expect("armed");
    wire.enter("a");
    std::fs::write(elsewhere.join("deep/table.csv"), "a,b\n9,9\n").unwrap();
    std::fs::write(elsewhere.join("summary.json"), "{}").unwrap();
    let error = wire
        .leave("a")
        .expect_err("a change elsewhere fails the branch");
    assert!(error.contains("table.csv restored"), "{error}");
    assert_eq!(
        std::fs::read_to_string(elsewhere.join("deep/table.csv")).unwrap(),
        "a,b\n1,2\n"
    );
    assert!(!elsewhere.join("summary.json").exists());
    assert!(spill.exists());
    drop(wire);
    assert!(!spill.exists(), "the spilled bytes are removed afterwards");
}

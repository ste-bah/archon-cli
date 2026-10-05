//! Issue 328: one baseline. A freeze proves its checks on a tree and records
//! it in the acceptance lock; every round, and every later freeze, proves on
//! that recorded tree. Only a lock that records none falls back, with a note.

use archon_workflow::task_set_contract::{ACCEPTANCE_LOCK_FILE, TrustedCwd};

use super::probe_tests::{git, trees};
use super::{Baseline, ExecutabilityProbe, HostProbe};

/// Record `commit` as the freeze's baseline in the task set's lock.
fn record(tasks: &std::path::Path, commit: &str) {
    let path = tasks.join(ACCEPTANCE_LOCK_FILE);
    let mut lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    lock["baseline_commit"] = serde_json::Value::String(commit.to_string());
    std::fs::write(&path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
}

/// A commit after HEAD, standing for the run's base once work has landed.
fn later(repo: &std::path::Path) -> String {
    git(repo, &["commit", "-q", "--allow-empty", "-m", "landed"]);
    git(repo, &["rev-parse", "HEAD"])
}

#[test]
fn a_round_proves_on_the_commit_its_freeze_recorded() {
    let trees = trees(&[("AC-1", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let run_base = later(&trees.repo);
    // repository.lock names `base`; the freeze recorded `head`.
    record(&trees.set.tasks, &trees.head);
    let (baseline, note) = Baseline::for_round(&trees.repo, &trees.set.tasks, Some(&run_base));
    assert_eq!(baseline.map(|b| b.commit), Some(trees.head.clone()));
    assert_eq!(note, None, "the recorded baseline needs no note");
    // A freeze run again proves on the same recorded tree.
    let freeze = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    assert_eq!(freeze.baseline_commit(), Some(trees.head.clone()));
}

#[test]
fn a_lock_that_records_no_baseline_falls_back_to_the_freezes_own_rule_with_a_note() {
    let trees = trees(&[("AC-1", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let run_base = later(&trees.repo);
    let (baseline, note) = Baseline::for_round(&trees.repo, &trees.set.tasks, Some(&run_base));
    assert_eq!(
        baseline.map(|b| b.commit),
        Some(trees.base.clone()),
        "the decomposition's commit, the one the freeze proved on, never the run's base"
    );
    let note = note.expect("a fallback is noted");
    assert!(note.contains("recorded no baseline"), "{note}");
    assert!(note.contains(&trees.base), "{note}");
    // A recorded commit that is not in the repository is noted too.
    record(&trees.set.tasks, &"0".repeat(40));
    let (baseline, note) = Baseline::for_round(&trees.repo, &trees.set.tasks, Some(&run_base));
    assert_eq!(baseline.map(|b| b.commit), Some(trees.base.clone()));
    assert!(note.is_some_and(|note| note.contains("is not a commit of")));
    // With no decomposition record either, the run's base, noted.
    std::fs::remove_file(trees.set.tasks.join("repository.lock")).unwrap();
    let (baseline, note) = Baseline::for_round(&trees.repo, &trees.set.tasks, Some(&run_base));
    assert_eq!(baseline.map(|b| b.commit), Some(run_base));
    assert!(note.is_some_and(|note| note.contains("the run's base commit")));
}

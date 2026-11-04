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

/// Without its git history a round still finds its recorded baseline, by
/// id, so a proof recorded for that id is reused (Issues 219, 328).
#[test]
fn a_recorded_baseline_is_found_without_git() {
    let trees = trees(&[("AC-1", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let run_base = later(&trees.repo);
    std::fs::rename(trees.repo.join(".git"), trees.repo.join("git-moved")).unwrap();
    let (baseline, _) = Baseline::for_round(&trees.repo, &trees.set.tasks, Some(&run_base));
    assert_eq!(baseline.map(|b| b.commit), Some(trees.base.clone()));
    std::fs::remove_file(trees.set.tasks.join("repository.lock")).unwrap();
    let (baseline, note) = Baseline::for_round(&trees.repo, &trees.set.tasks, Some(&run_base));
    assert_eq!(baseline.map(|b| b.commit), Some(run_base));
    assert!(note.is_some_and(|note| note.contains("the run's base commit")));
    record(&trees.set.tasks, &trees.head);
    let (baseline, note) = Baseline::for_round(&trees.repo, &trees.set.tasks, None);
    assert_eq!(
        (baseline.map(|b| b.commit), note),
        (Some(trees.head.clone()), None)
    );
}

#[tokio::test]
#[allow(non_snake_case)]
async fn P2_15_only_assertion_failures_hold_original_strength() {
    use std::os::unix::fs::PermissionsExt;
    let trees = trees(&[("check", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let external = tempfile::tempdir().unwrap();
    std::fs::write(external.path().join("tool"), "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(
        external.path().join("tool"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let result = |exit_code, stderr: &str, operational_error| {
        archon_workflow::acceptance_scratch::CheckResult {
            acceptance_id: "check".into(),
            exit_code,
            stderr: stderr.as_bytes().to_vec(),
            stdout: Vec::new(),
            verdict_streams: Default::default(),
            fault: None,
            quota_walk_count: 0,
            operational_error,
            classification: None,
            environment_note: None,
        }
    };
    for (command, result, call, expected) in [
        (
            "test -f feature.txt",
            result(Some(0), "", None),
            "",
            super::Original::Passed,
        ),
        (
            "test -f feature.txt",
            result(Some(1), "assertion failed", None),
            "",
            super::Original::Failed,
        ),
        (
            "python3 -c 'broken('",
            result(
                Some(1),
                "  File \"<string>\", line 1\n    broken(\nSyntaxError: invalid syntax\n",
                None,
            ),
            "",
            super::Original::Defect,
        ),
        (
            "test -f feature.txt",
            result(None, "", Some("host unavailable".into())),
            "",
            super::Original::Defect,
        ),
        (
            "tool --bad",
            result(
                Some(2),
                "error: unexpected argument '--bad' found\nUsage: tool [OPTIONS]\n",
                None,
            ),
            "",
            super::Original::Defect,
        ),
        (
            "test -f feature.txt",
            result(Some(1), "failure", None),
            "acceptance-passability-batch:check",
            super::Original::Defect,
        ),
    ] {
        let mut contract = trees.contract();
        contract.acceptance[0].check =
            archon_workflow::task_set_contract::AcceptanceCheck::Command {
                command: command.into(),
                cwd: TrustedCwd::RepoRoot,
            };
        contract.acceptance[0].judgment.host_call_id = call.into();
        if !call.is_empty() {
            contract.acceptance[0].judgment.verdict =
                archon_workflow::task_set_contract::JudgeDecision::Refuted;
        }
        let at = super::verdict::Context::new(
            [("PATH".into(), external.path().display().to_string())].into(),
            &contract,
        )
        .with_requirements(&probe, &contract);
        assert_eq!(
            super::originals(&at, &contract, vec![result]).await["check"],
            expected,
            "{command}"
        );
    }
}

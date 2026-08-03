//! What is tested here is the part that can lose someone's work.
//!
//! The refusals and the restore are exercised against real files in a real
//! temporary git repository; the verifier itself is not run, because a test that
//! shelled out to `cargo` to prove a mutation broke a build would be the
//! workspace-wide test run NFR-004 forbids, wearing a different hat.

use std::path::Path;

use archon_knowledge::traceability::anchors::file_hash;
use archon_knowledge::traceability::{FalsificationPlan, MutationKind, RefusedToRun};

use super::*;

/// A repository with one committed Rust file, so the cleanliness check has
/// something true to say.
fn repo_with_committed_file(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    std::fs::create_dir_all(root.join("src")).expect("mkdir");
    std::fs::write(root.join("src/a.rs"), body).expect("write");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "src/a.rs"],
        vec!["-c", "commit.gpgsign=false", "commit", "-qm", "seed"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .expect("git");
        assert!(status.status.success(), "git failed: {status:?}");
    }
    let path = root.join("src/a.rs");
    (dir, path)
}

fn plan_for(root: &Path, command: &str) -> FalsificationPlan {
    let bytes = std::fs::read(root.join("src/a.rs")).expect("read");
    FalsificationPlan {
        requirement_id: "REQ-DL-100".into(),
        severity_evidence: "fail closed".into(),
        task_id: "TASK-A".into(),
        file_path: "src/a.rs".into(),
        line_start: 2,
        line_end: 2,
        expected_file_hash: file_hash(&bytes),
        mutation: MutationKind::AbortAnchoredRange,
        command: command.into(),
    }
}

#[test]
fn the_guard_restores_the_file_and_removes_its_backup_when_dropped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("a.rs");
    let original = b"fn a() {}\n".to_vec();
    std::fs::write(&path, &original).expect("write");

    let backup = {
        let installed =
            guard::MutationGuard::install(&path, original.clone(), b"BOOM\n").expect("install");
        let backup = installed.backup_file().to_path_buf();
        assert_eq!(std::fs::read(&path).expect("read"), b"BOOM\n");
        assert_eq!(std::fs::read(&backup).expect("read"), original);
        backup
        // `installed` drops here — no explicit restore call anywhere above.
    };

    assert_eq!(std::fs::read(&path).expect("read"), original);
    assert!(!backup.exists(), "the backup outlived the restore");
}

/// The path a `?` or a failing verifier takes, and the one a panic takes. Both
/// end in the same place because both unwind.
#[test]
fn the_guard_restores_through_a_panic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("a.rs");
    let original = b"fn a() {}\n".to_vec();
    std::fs::write(&path, &original).expect("write");

    let panicked = std::panic::catch_unwind({
        let path = path.clone();
        let original = original.clone();
        move || {
            let _installed =
                guard::MutationGuard::install(&path, original, b"BOOM\n").expect("install");
            panic!("verifier harness exploded mid-run");
        }
    });
    assert!(panicked.is_err(), "the panic must not be swallowed");
    assert_eq!(std::fs::read(&path).expect("read"), original);
    assert!(!guard::backup_path(&path).exists());
}

#[test]
fn restoring_twice_is_a_no_op_rather_than_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("a.rs");
    std::fs::write(&path, b"fn a() {}\n").expect("write");
    let mut installed =
        guard::MutationGuard::install(&path, b"fn a() {}\n".to_vec(), b"BOOM\n").expect("install");
    installed.restore().expect("first restore");
    // The explicit call and the one in `Drop` must not fight over the backup.
    installed.restore().expect("second restore");
    assert_eq!(std::fs::read(&path).expect("read"), b"fn a() {}\n");
}

#[test]
fn a_workspace_wide_verifier_is_refused_before_anything_is_written() {
    let (dir, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
    let before = std::fs::read(&path).expect("read");
    for command in [
        "cargo test --workspace",
        "cargo test --all",
        // Unscoped, so workspace-wide in effect even though it never says so.
        "cargo test",
        "cargo nextest run",
    ] {
        let plan = plan_for(dir.path(), command);
        let refused = attempt(dir.path(), &plan).expect_err("refused");
        assert!(
            matches!(refused, RefusedToRun::WorkspaceWideCommand { .. }),
            "{command}: {refused:?}"
        );
        assert!(refused.describe().contains("NFR-004"), "{command}");
    }
    assert_eq!(std::fs::read(&path).expect("read"), before);
    assert!(!guard::backup_path(&path).exists());
}

#[test]
fn a_scoped_verifier_passes_the_command_vet() {
    // The counterpart to the refusals above: the vet must not refuse everything.
    assert!(verifier::vet("cargo test -p archon-knowledge falsification").is_ok());
    assert!(verifier::vet("cargo nextest run -p archon-trading").is_ok());
    assert!(verifier::vet("pytest tests/test_interval.py").is_ok());
}

#[test]
fn a_verifier_that_needs_a_shell_is_refused_rather_than_reinterpreted() {
    let refused = verifier::vet("cargo test -p x && echo done").expect_err("refused");
    assert!(
        matches!(refused, RefusedToRun::NotDirectlyExecutable { .. }),
        "{refused:?}"
    );
    assert!(verifier::vet("cargo test -p x | tee log").is_err());
    assert!(verifier::vet("cargo test -p x > log").is_err());
}

#[test]
fn a_dirty_target_file_is_refused_and_left_alone() {
    let (dir, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
    std::fs::write(&path, "fn a() {}\nfn b() { /* local edit */ }\n").expect("write");

    let plan = FalsificationPlan {
        // The plan is re-hashed against the edited file so the refusal under
        // test is cleanliness and not staleness.
        expected_file_hash: file_hash(&std::fs::read(&path).expect("read")),
        ..plan_for(dir.path(), "cargo test -p archon-knowledge")
    };
    let refused = attempt(dir.path(), &plan).expect_err("refused");
    assert!(
        matches!(refused, RefusedToRun::DirtyFile { .. }),
        "{refused:?}"
    );
    assert!(refused.describe().contains("cannot be safely restored"));
    // Refused means refused: no backup, no mutation, no verifier run.
    assert!(!guard::backup_path(&path).exists());
    assert!(
        std::fs::read_to_string(&path)
            .expect("read")
            .contains("local edit")
    );
}

#[test]
fn a_tree_that_is_not_a_repository_is_treated_as_dirty() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    std::fs::write(dir.path().join("src/a.rs"), "fn a() {}\nfn b() {}\n").expect("write");

    let plan = plan_for(dir.path(), "cargo test -p archon-knowledge");
    let refused = attempt(dir.path(), &plan).expect_err("refused");
    assert!(
        matches!(refused, RefusedToRun::CleanlinessUnknown { .. }),
        "{refused:?}"
    );
    assert!(refused.describe().contains("treated as dirty"));
}

#[test]
fn a_file_that_changed_since_the_plan_is_refused() {
    let (dir, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
    let plan = FalsificationPlan {
        expected_file_hash: "0".repeat(64),
        ..plan_for(dir.path(), "cargo test -p archon-knowledge")
    };
    let refused = attempt(dir.path(), &plan).expect_err("refused");
    assert!(
        matches!(refused, RefusedToRun::FileChangedSincePlan { .. }),
        "{refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "fn a() {}\nfn b() {}\n"
    );
}

#[test]
fn a_stranded_backup_stops_the_next_run_instead_of_being_overwritten() {
    let (dir, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
    std::fs::write(guard::backup_path(&path), b"someone else's original\n").expect("write");

    let plan = plan_for(dir.path(), "cargo test -p archon-knowledge");
    let refused = attempt(dir.path(), &plan).expect_err("refused");
    assert!(
        matches!(refused, RefusedToRun::UnreconciledBackup { .. }),
        "{refused:?}"
    );
    assert!(refused.describe().contains("restore by hand"));
    // The stranded copy is still there: reconciling it is the human's call.
    assert_eq!(
        std::fs::read(guard::backup_path(&path)).expect("read"),
        b"someone else's original\n"
    );
}

#[test]
fn a_language_with_no_abort_form_is_refused_before_the_file_is_read() {
    let (dir, _path) = repo_with_committed_file("a\nb\n");
    std::fs::write(dir.path().join("src/notes.txt"), "a\nb\n").expect("write");
    let plan = FalsificationPlan {
        file_path: "src/notes.txt".into(),
        ..plan_for(dir.path(), "cargo test -p archon-knowledge")
    };
    let refused = attempt(dir.path(), &plan).expect_err("refused");
    assert!(
        matches!(refused, RefusedToRun::NoMutationForLanguage { .. }),
        "{refused:?}"
    );
}

#[test]
fn a_build_failure_in_the_mutated_run_is_inconclusive_not_a_kill() {
    let outcome = classify(
        "rust",
        verifier::Ran::Finished {
            code: Some(101),
            success: false,
            output: "error[E0308]: mismatched types\nerror: could not compile `x`".into(),
        },
    );
    assert!(
        matches!(
            outcome,
            FalsificationOutcome::Inconclusive(Inconclusive::MutantDidNotBuild { .. })
        ),
        "{outcome:?}"
    );
}

#[test]
fn a_test_failure_in_the_mutated_run_is_the_one_thing_that_promotes() {
    let outcome = classify(
        "rust",
        verifier::Ran::Finished {
            code: Some(101),
            success: false,
            output: "test interval::rejects_unknown ... FAILED\ntest result: FAILED".into(),
        },
    );
    assert_eq!(
        outcome,
        FalsificationOutcome::DependencyShown {
            mutated_exit: Some(101)
        }
    );
}

#[test]
fn a_verifier_that_still_passes_while_mutated_is_recorded_as_decoration() {
    let outcome = classify(
        "rust",
        verifier::Ran::Finished {
            code: Some(0),
            success: true,
            output: "test result: ok. 12 passed".into(),
        },
    );
    assert_eq!(
        outcome,
        FalsificationOutcome::EdgeIsDecoration {
            mutated_exit: Some(0)
        }
    );
}

#[test]
fn a_timeout_and_an_unlaunchable_verifier_both_decide_nothing() {
    assert!(matches!(
        classify("rust", verifier::Ran::TimedOut { seconds: 900 }),
        FalsificationOutcome::Inconclusive(Inconclusive::TimedOut { .. })
    ));
    assert!(matches!(
        classify(
            "rust",
            verifier::Ran::NotLaunchable {
                reason: "no such file".into()
            }
        ),
        FalsificationOutcome::Inconclusive(Inconclusive::VerifierNotLaunchable { .. })
    ));
}

/// The verifier is real here — `git --version` is cheap, deterministic, and
/// present, since the cleanliness check already requires it. What is under test
/// is that a verifier which passes both times is read as decoration rather than
/// as a kill, and that the file comes back untouched afterwards.
#[test]
fn an_end_to_end_run_whose_verifier_ignores_the_mutation_reports_decoration() {
    let (dir, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
    let before = std::fs::read(&path).expect("read");
    let plan = plan_for(dir.path(), "git --version");

    let outcome = attempt(dir.path(), &plan).expect("ran");
    assert!(
        matches!(outcome, FalsificationOutcome::EdgeIsDecoration { .. }),
        "{outcome:?}"
    );
    // Promotes nothing, and leaves the tree exactly as it found it.
    assert_eq!(
        outcome.level_after(archon_knowledge::traceability::ProofLevel::Exercised),
        archon_knowledge::traceability::ProofLevel::Exercised
    );
    assert_eq!(std::fs::read(&path).expect("read"), before);
    assert!(!guard::backup_path(&path).exists());
}

/// The whole pipeline, including the only promotion it can produce.
///
/// `git diff --quiet -- src/a.rs` is a real verifier that is genuinely sensitive
/// to the anchored bytes: exit 0 while the file matches HEAD, exit 1 once it
/// does not. It stands in for `cargo test -p …` without building anything,
/// which is the point — a test that compiled a crate twice to watch a mutation
/// break it would be the disk-exhausting run NFR-004 forbids.
#[test]
fn a_verifier_that_breaks_with_the_anchored_lines_promotes_the_edge_to_falsifiable() {
    use archon_knowledge::traceability::anchors::{Anchor, AnchorFreshness};
    use archon_knowledge::traceability::report::{AnchorVerdict, RequirementRow};
    use archon_knowledge::traceability::{CoverageReport, ProofLevel, Severity, TraceReport};

    let (dir, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
    let before = std::fs::read(&path).expect("read");
    let plan = plan_for(dir.path(), "git diff --quiet -- src/a.rs");

    let verdict = AnchorVerdict {
        anchor: Anchor {
            requirement_id: plan.requirement_id.clone(),
            task_id: plan.task_id.clone(),
            file_path: plan.file_path.clone(),
            line_start: plan.line_start,
            line_end: plan.line_end,
            file_hash: plan.expected_file_hash.clone(),
            path_scope: "src/".into(),
            relevance_score: 0.9,
        },
        freshness: AnchorFreshness::Fresh,
        level: ProofLevel::Exercised,
        proof: None,
        missing: None,
        falsification: Ok(plan),
        falsification_outcome: None,
    };
    let mut report = TraceReport {
        prd_path: "PRD.md".into(),
        task_dir: "tasks".into(),
        coverage: CoverageReport::default(),
        rows: vec![RequirementRow {
            requirement_id: "REQ-DL-100".into(),
            prd_line: 1,
            severity: Severity::Error,
            severity_evidence: Some("fail closed".into()),
            claimed_by: vec!["TASK-A".into()],
            anchors: vec![verdict],
            anchor_gap: None,
            level: ProofLevel::Exercised,
        }],
        shared_anchors: Vec::new(),
        stale_anchors: 0,
        index_consulted: true,
    };

    execute_plans(dir.path(), &mut report);

    assert_eq!(
        report.rows[0].anchors[0].falsification_outcome,
        Some(FalsificationOutcome::DependencyShown {
            mutated_exit: Some(1)
        })
    );
    assert_eq!(report.rows[0].anchors[0].level, ProofLevel::Falsifiable);
    // The row level is recomputed from its anchors, not left where it started.
    assert_eq!(report.rows[0].level, ProofLevel::Falsifiable);
    assert!(report.rows[0].satisfied());
    // And the tree is exactly as it was found.
    assert_eq!(std::fs::read(&path).expect("read"), before);
    assert!(!guard::backup_path(&path).exists());
}

/// A verifier that fails on the unmodified tree is refused before the mutation,
/// so a pre-existing breakage can never be mistaken for a kill.
#[test]
fn a_baseline_that_does_not_pass_is_refused_before_the_mutation() {
    let (dir, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
    let before = std::fs::read(&path).expect("read");
    let plan = plan_for(dir.path(), "git rev-parse --verify no-such-ref");

    let refused = attempt(dir.path(), &plan).expect_err("refused");
    assert!(
        matches!(refused, RefusedToRun::BaselineDidNotPass { .. }),
        "{refused:?}"
    );
    assert!(refused.describe().contains("already failing"));
    assert_eq!(std::fs::read(&path).expect("read"), before);
    assert!(!guard::backup_path(&path).exists());
}

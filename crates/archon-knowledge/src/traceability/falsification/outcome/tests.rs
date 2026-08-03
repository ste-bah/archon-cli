use super::*;

#[test]
fn only_a_killed_mutant_promotes_and_only_from_exercised() {
    let killed = FalsificationOutcome::DependencyShown {
        mutated_exit: Some(101),
    };
    assert_eq!(
        killed.level_after(ProofLevel::Exercised),
        ProofLevel::Falsifiable
    );
    // No second route to the top of the ladder: an edge that never had a
    // passing verifier cannot arrive at Falsifiable by experiment.
    assert_eq!(
        killed.level_after(ProofLevel::Candidate),
        ProofLevel::Candidate
    );
    assert_eq!(
        killed.level_after(ProofLevel::Unproven),
        ProofLevel::Unproven
    );
}

#[test]
fn a_surviving_mutant_neither_promotes_nor_demotes() {
    let survived = FalsificationOutcome::EdgeIsDecoration {
        mutated_exit: Some(0),
    };
    // Exercised stays: the recorded run really did pass and really did read the
    // file. What is refuted is dependency, and that is said in words instead.
    assert_eq!(
        survived.level_after(ProofLevel::Exercised),
        ProofLevel::Exercised
    );
    let text = survived.describe();
    assert!(text.contains("DECORATION"), "{text}");
    assert!(text.contains("does not promote"), "{text}");
}

#[test]
fn every_non_kill_outcome_leaves_the_level_untouched() {
    let untouched = [
        FalsificationOutcome::Inconclusive(Inconclusive::TimedOut { seconds: 900 }),
        FalsificationOutcome::Inconclusive(Inconclusive::MutantDidNotBuild {
            marker: "error[E".into(),
        }),
        FalsificationOutcome::Inconclusive(Inconclusive::UnclassifiableFailure {
            language: "cobol".into(),
        }),
        FalsificationOutcome::Refused(RefusedToRun::DirtyFile {
            file_path: "src/a.rs".into(),
            status: " M src/a.rs".into(),
        }),
        FalsificationOutcome::Refused(RefusedToRun::WorkspaceWideCommand {
            command: "cargo test --workspace".into(),
            token: "--workspace".into(),
        }),
    ];
    for outcome in untouched {
        assert_eq!(
            outcome.level_after(ProofLevel::Exercised),
            ProofLevel::Exercised,
            "{outcome:?}"
        );
        assert!(outcome.describe().contains("No promotion"), "{outcome:?}");
    }
}

#[test]
fn a_build_failure_is_never_read_as_a_kill() {
    let outcome = FalsificationOutcome::Inconclusive(Inconclusive::MutantDidNotBuild {
        marker: "error: could not compile".into(),
    });
    assert_eq!(
        outcome.level_after(ProofLevel::Exercised),
        ProofLevel::Exercised
    );
    let text = outcome.describe();
    assert!(text.contains("depended on by compilation"), "{text}");
}

#[test]
fn refusals_name_the_specific_absent_fact() {
    let dirty = RefusedToRun::DirtyFile {
        file_path: "src/a.rs".into(),
        status: " M src/a.rs".into(),
    };
    assert!(dirty.describe().contains("cannot be safely restored"));

    let wide = RefusedToRun::WorkspaceWideCommand {
        command: "cargo test --workspace".into(),
        token: "--workspace".into(),
    };
    assert!(wide.describe().contains("NFR-004"), "{}", wide.describe());

    let baseline = RefusedToRun::BaselineDidNotPass {
        command: "cargo test -p x".into(),
        exit_code: Some(101),
    };
    assert!(baseline.describe().contains("already failing"));

    let stranded = RefusedToRun::UnreconciledBackup {
        backup_path: "src/a.rs.archon-falsify-backup".into(),
    };
    assert!(stranded.describe().contains("restore by hand"));
}

#[test]
fn a_language_with_no_marker_list_gets_no_verdict() {
    assert!(
        build_failure_markers("rust")
            .expect("rust")
            .contains(&"error[E")
    );
    assert!(build_failure_markers("python").is_some());
    assert!(build_failure_markers("go").is_some());
    assert!(build_failure_markers("typescript").is_some());
    // Not a guess: an unlisted language is unclassifiable, which is a refusal
    // to promote rather than a default.
    assert!(build_failure_markers("cobol").is_none());
}

use super::*;
use crate::traceability::ladder::ReadScope;
use crate::traceability::requirements::extract_requirements;
use crate::traceability::tasks::VerifierOrigin;

fn requirement(line: &str) -> Requirement {
    extract_requirements(line).pop().expect("one requirement")
}

fn anchor() -> Anchor {
    Anchor {
        requirement_id: "REQ-DL-131".into(),
        task_id: "TASK-A".into(),
        file_path: "src/interval.rs".into(),
        line_start: 40,
        line_end: 55,
        file_hash: "0123456789abcdef0123456789abcdef".into(),
        path_scope: "src/interval.rs".into(),
        relevance_score: 0.7,
    }
}

fn proof() -> ExercisedProof {
    ExercisedProof {
        command: "cargo test -p archon-trading interval".into(),
        origin: VerifierOrigin::FocusedTests,
        read_scope: ReadScope::Node("TASK-A".into()),
        read_path: "src/interval.rs".into(),
    }
}

#[test]
fn plans_only_for_error_severity_requirements() {
    let out_of_scope = requirement("- REQ-DL-060: Add CLI commands for data status.\n");
    assert_eq!(
        plan(
            &out_of_scope,
            &anchor(),
            ProofLevel::Exercised,
            Some(&proof())
        ),
        Err(NotPlannable::OutOfSeverityScope)
    );
}

#[test]
fn plans_only_once_the_edge_is_exercised() {
    let in_scope = requirement("- REQ-DL-131: Unknown native interval status must fail closed.\n");
    assert_eq!(
        plan(&in_scope, &anchor(), ProofLevel::Candidate, None),
        Err(NotPlannable::NotYetExercised(ProofLevel::Candidate))
    );
    // Even at Exercised, a missing proof record leaves nothing to break.
    assert!(plan(&in_scope, &anchor(), ProofLevel::Exercised, None).is_err());
}

#[test]
fn a_plan_names_file_lines_hash_command_and_the_severity_phrase() {
    let in_scope = requirement("- REQ-DL-131: Unknown native interval status must fail closed.\n");
    let plan = plan(&in_scope, &anchor(), ProofLevel::Exercised, Some(&proof())).expect("plan");
    assert_eq!(plan.requirement_id, "REQ-DL-131");
    assert_eq!(plan.severity_evidence, "fail closed");
    assert_eq!(plan.file_path, "src/interval.rs");
    assert_eq!((plan.line_start, plan.line_end), (40, 55));
    assert_eq!(plan.expected_file_hash, anchor().file_hash);
    assert_eq!(plan.command, "cargo test -p archon-trading interval");
    assert_eq!(plan.mutation, MutationKind::AbortAnchoredRange);
}

#[test]
fn the_pass_criterion_requires_failure_then_restoration() {
    let in_scope = requirement("- REQ-DL-100: any `error` check must have status=failed.\n");
    let plan = plan(&in_scope, &anchor(), ProofLevel::Exercised, Some(&proof())).expect("plan");
    let criterion = plan.pass_criterion();
    assert!(criterion.contains("must FAIL"), "{criterion}");
    assert!(criterion.contains("must PASS again"), "{criterion}");
    assert!(criterion.contains("40-55"), "{criterion}");
}

#[test]
fn a_recipe_exists_only_for_languages_with_a_known_abort_form() {
    let in_scope = requirement("- REQ-DL-131: must fail closed.\n");
    let plan = plan(&in_scope, &anchor(), ProofLevel::Exercised, Some(&proof())).expect("plan");
    let rust = plan.shell_recipe("rust").expect("rust recipe");
    assert_eq!(rust.len(), 5);
    assert!(rust[2].contains("unreachable!"), "{:?}", rust[2]);
    assert!(rust[3].contains("NON-ZERO"), "{:?}", rust[3]);
    // No guessed abort form for an unknown language: no plan rather than a
    // mutation that does not compile.
    assert!(plan.shell_recipe("cobol").is_none());
}

#[test]
fn not_plannable_explains_itself_in_one_line() {
    assert!(
        NotPlannable::OutOfSeverityScope
            .describe()
            .contains("per validation check")
    );
    assert!(
        NotPlannable::NotYetExercised(ProofLevel::Candidate)
            .describe()
            .contains("Candidate")
    );
}

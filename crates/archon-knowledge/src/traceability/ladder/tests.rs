use super::*;
use crate::traceability::tasks::{FocusedTestEntry, VerifierCommand};

fn anchor(req: &str, path: &str) -> Anchor {
    Anchor {
        requirement_id: req.into(),
        task_id: "TASK-A".into(),
        file_path: path.into(),
        line_start: 10,
        line_end: 20,
        file_hash: "hash".into(),
        path_scope: path.into(),
        relevance_score: 0.9,
    }
}

fn binding(commands: &[&str]) -> TaskBinding {
    TaskBinding {
        task_id: "TASK-A".into(),
        source_path: "tests/TASK-A.md".into(),
        implements: vec!["REQ-DL-001".into()],
        path_scopes: vec!["src/a.rs".into()],
        focused_tests: commands
            .iter()
            .map(|c| FocusedTestEntry::Command((*c).to_string()))
            .collect(),
        verifier_commands: commands
            .iter()
            .map(|c| VerifierCommand {
                command: (*c).to_string(),
                origin: VerifierOrigin::FocusedTests,
            })
            .collect(),
    }
}

fn ran(command: &str) -> CommandEvidence {
    CommandEvidence {
        command: command.into(),
        succeeded: true,
        exit_code: Some(0),
    }
}

fn read(node: &str, path: &str) -> ReadEvidence {
    ReadEvidence {
        node_id: node.into(),
        file_path: path.into(),
    }
}

#[test]
fn only_the_top_two_levels_satisfy_a_gate() {
    assert!(!ProofLevel::Unproven.satisfies_promotion_gate());
    assert!(!ProofLevel::Candidate.satisfies_promotion_gate());
    assert!(ProofLevel::Exercised.satisfies_promotion_gate());
    assert!(ProofLevel::Falsifiable.satisfies_promotion_gate());
    assert!(ProofLevel::Candidate < ProofLevel::Exercised);
}

#[test]
fn passing_verifier_plus_a_node_scoped_read_promotes() {
    let (level, proof, missing) = promote(
        &anchor("REQ-DL-001", "src/a.rs"),
        &binding(&["cargo test -p x"]),
        &[ran("cargo test -p x")],
        &[read("TASK-A", "src/a.rs")],
    );
    assert_eq!(level, ProofLevel::Exercised);
    assert!(missing.is_none());
    let proof = proof.expect("proof");
    assert_eq!(proof.command, "cargo test -p x");
    assert_eq!(proof.read_scope, ReadScope::Node("TASK-A".into()));
    assert_eq!(proof.read_path, "src/a.rs");
}

#[test]
fn a_read_attributed_to_another_node_promotes_but_records_the_weaker_scope() {
    let (level, proof, _) = promote(
        &anchor("REQ-DL-001", "src/a.rs"),
        &binding(&["cargo test -p x"]),
        &[ran("cargo test -p x")],
        &[read("__root__", "src/a.rs")],
    );
    assert_eq!(level, ProofLevel::Exercised);
    assert_eq!(proof.expect("proof").read_scope, ReadScope::Run);
}

/// The F1 case, reproduced as a test: one command's evidence reused across four
/// requirements. Only the anchor the run actually read may promote.
#[test]
fn one_commands_evidence_cannot_promote_four_unrelated_anchors() {
    let task = binding(&["cargo test -p x"]);
    let commands = [ran("cargo test -p x")];
    let reads = [read("TASK-A", "src/one.rs")];

    let promoted: Vec<ProofLevel> = ["src/one.rs", "src/two.rs", "src/three.rs", "src/four.rs"]
        .iter()
        .enumerate()
        .map(|(i, path)| {
            promote(
                &anchor(&format!("REQ-DL-00{}", i + 1), path),
                &task,
                &commands,
                &reads,
            )
            .0
        })
        .collect();

    assert_eq!(
        promoted,
        vec![
            ProofLevel::Exercised,
            ProofLevel::Candidate,
            ProofLevel::Candidate,
            ProofLevel::Candidate,
        ]
    );
    assert_eq!(
        promoted
            .iter()
            .filter(|l| l.satisfies_promotion_gate())
            .count(),
        1
    );
}

#[test]
fn a_passing_command_that_never_read_the_anchor_names_that_exactly() {
    let (level, _, missing) = promote(
        &anchor("REQ-DL-002", "src/two.rs"),
        &binding(&["cargo test -p x"]),
        &[ran("cargo test -p x")],
        &[read("TASK-A", "src/one.rs")],
    );
    assert_eq!(level, ProofLevel::Candidate);
    let missing = missing.expect("missing");
    assert_eq!(
        missing,
        MissingForPromotion::AnchorNotRead {
            command: "cargo test -p x".into(),
            anchor_path: "src/two.rs".into(),
        }
    );
    assert!(missing.describe().contains("src/two.rs"));
}

#[test]
fn a_failing_declared_verifier_does_not_promote() {
    let (level, _, missing) = promote(
        &anchor("REQ-DL-001", "src/a.rs"),
        &binding(&["cargo test -p x"]),
        &[CommandEvidence {
            command: "cargo test -p x".into(),
            succeeded: false,
            exit_code: Some(101),
        }],
        &[read("TASK-A", "src/a.rs")],
    );
    assert_eq!(level, ProofLevel::Candidate);
    assert_eq!(
        missing.expect("missing"),
        MissingForPromotion::DeclaredCommandFailed {
            command: "cargo test -p x".into(),
            exit_code: Some(101),
        }
    );
}

#[test]
fn succeeded_with_a_nonzero_exit_code_is_self_contradictory_and_proves_nothing() {
    let contradictory = CommandEvidence {
        command: "cargo test -p x".into(),
        succeeded: true,
        exit_code: Some(1),
    };
    assert!(!contradictory.passed());
    let (level, ..) = promote(
        &anchor("REQ-DL-001", "src/a.rs"),
        &binding(&["cargo test -p x"]),
        &[contradictory],
        &[read("TASK-A", "src/a.rs")],
    );
    assert_eq!(level, ProofLevel::Candidate);
}

#[test]
fn an_undeclared_command_cannot_promote_however_well_it_ran() {
    let (level, _, missing) = promote(
        &anchor("REQ-DL-001", "src/a.rs"),
        &binding(&["cargo test -p x"]),
        &[ran("cargo test --workspace")],
        &[read("TASK-A", "src/a.rs")],
    );
    assert_eq!(level, ProofLevel::Candidate);
    assert_eq!(
        missing.expect("missing"),
        MissingForPromotion::NoDeclaredCommandRan {
            declared: vec!["cargo test -p x".into()],
        }
    );
}

#[test]
fn command_matching_ignores_only_whitespace() {
    let (level, ..) = promote(
        &anchor("REQ-DL-001", "src/a.rs"),
        &binding(&["cargo   test -p x"]),
        &[ran("cargo test  -p  x")],
        &[read("TASK-A", "src/a.rs")],
    );
    assert_eq!(level, ProofLevel::Exercised);
}

#[test]
fn a_task_whose_focused_tests_are_prose_names_that_as_the_gap() {
    let mut task = binding(&[]);
    task.focused_tests = vec![
        FocusedTestEntry::Prose("Registry schema migration test.".into()),
        FocusedTestEntry::Prose("Atomic write behaviour test.".into()),
    ];
    let (level, _, missing) = promote(
        &anchor("REQ-DL-001", "src/a.rs"),
        &task,
        &[ran("cargo test -p x")],
        &[read("TASK-A", "src/a.rs")],
    );
    assert_eq!(level, ProofLevel::Candidate);
    assert_eq!(
        missing.expect("missing"),
        MissingForPromotion::NoDeclaredVerifier { prose_entries: 2 }
    );
}

#[test]
fn an_absent_trace_proves_nothing_and_says_so() {
    let (level, _, missing) = promote(
        &anchor("REQ-DL-001", "src/a.rs"),
        &binding(&["cargo test -p x"]),
        &[],
        &[],
    );
    assert_eq!(level, ProofLevel::Candidate);
    assert_eq!(missing.expect("missing"), MissingForPromotion::NoTrace);
}

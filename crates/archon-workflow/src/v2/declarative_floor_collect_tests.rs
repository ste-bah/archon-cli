//! Issue-87: a declared deliverable that IS a directory.
//!
//! Live, a task declared a directory of run records. It held five run
//! directories, one with a 1560-byte report, and the floor reported
//! "declared deliverable missing or empty" every round — the shape was the
//! complaint, so no remediation could ever answer it. These pin the rule the
//! declared-artifact guard already states, now that the collector calls it.

use super::declarative_floor_collect::collect_declarative_floor_facts;
use super::deliverable_contract::ContractRoots;
use crate::task_universe::WorkflowV2DeliverableContract;

const DECLARED: &str = "artifacts/runs";

fn contract() -> WorkflowV2DeliverableContract {
    WorkflowV2DeliverableContract {
        kind: "run-artifacts".into(),
        artifact_path: DECLARED.into(),
        min_instances: 1,
        ..Default::default()
    }
}

/// Two roots, as live: the project artifact root then the repository.
fn facts_for(
    build: impl FnOnce(&std::path::Path),
) -> super::declarative_floor::DeclarativeFloorFacts {
    let project = tempfile::tempdir().expect("project");
    let repository = tempfile::tempdir().expect("repository");
    build(project.path());
    let roots = ContractRoots::new(
        project.path().to_string_lossy().to_string(),
        Some(&repository.path().to_string_lossy()),
    );
    collect_declarative_floor_facts(&roots, &contract()).expect("facts")
}

fn write(root: &std::path::Path, relative: &str, bytes: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, bytes).expect("file");
}

#[test]
fn a_directory_holding_a_non_empty_file_is_present_with_its_bytes() {
    let facts = facts_for(|root| write(root, "artifacts/runs/spec-1/report.json", "{\"a\":1}"));
    assert!(facts.artifact_present);
    assert_eq!(facts.artifact_byte_len, 7);
    // Nothing was read into memory: a directory has no bytes of its own.
    assert!(facts.artifact_json.is_none());
}

/// The live shape exactly: evidence nested one level down, beside siblings.
#[test]
fn the_evidence_may_be_nested_at_any_depth() {
    let facts = facts_for(|root| {
        std::fs::create_dir_all(root.join("artifacts/runs/spec-empty")).expect("dir");
        write(root, "artifacts/runs/spec-2/nested/equity.jsonl", "row\n");
    });
    assert!(facts.artifact_present);
    assert_eq!(facts.artifact_byte_len, 4);
}

/// The litter the guard exists to refuse still reads as absent, so an empty
/// declaration cannot buy acceptance.
#[test]
fn an_empty_directory_or_one_of_empty_files_is_still_absent() {
    let empty = facts_for(|root| {
        std::fs::create_dir_all(root.join("artifacts/runs")).expect("dir");
    });
    assert!(!empty.artifact_present, "empty directory");
    assert_eq!(empty.artifact_byte_len, 0);

    let hollow = facts_for(|root| {
        write(root, "artifacts/runs/spec-3/report.json", "");
        std::fs::create_dir_all(root.join("artifacts/runs/spec-4/deeper")).expect("dir");
    });
    assert!(!hollow.artifact_present, "only empty files");
    assert_eq!(hollow.artifact_byte_len, 0);
}

#[test]
fn a_regular_file_deliverable_is_unchanged_and_a_missing_one_is_still_absent() {
    let file = facts_for(|root| write(root, "artifacts/runs", "payload"));
    assert!(file.artifact_present);
    assert_eq!(file.artifact_byte_len, 7);
    assert!(file.artifact_json.is_none(), "no .json suffix, so text");

    let missing = facts_for(|_| {});
    assert!(!missing.artifact_present);
    assert_eq!(missing.artifact_byte_len, 0);
    // The roots are still reported, which is what the finding names.
    assert_eq!(missing.searched_roots.len(), 2);
}

/// The floor's own verdict, end to end, on the live shape.
#[test]
fn the_floor_passes_a_directory_of_output_and_fails_an_empty_one() {
    use super::declarative_floor::{DeclarativeFloorEvaluation, evaluate_declarative_floor};
    let full = facts_for(|root| write(root, "artifacts/runs/spec-5/report.json", "{}"));
    assert_eq!(
        evaluate_declarative_floor(&contract(), &full),
        DeclarativeFloorEvaluation::Passed
    );
    let empty = facts_for(|root| {
        std::fs::create_dir_all(root.join("artifacts/runs")).expect("dir");
    });
    let DeclarativeFloorEvaluation::Failed { findings } =
        evaluate_declarative_floor(&contract(), &empty)
    else {
        panic!("an empty directory must still fail");
    };
    assert!(
        findings[0].starts_with("declared deliverable missing or empty: artifacts/runs"),
        "{findings:?}"
    );
}

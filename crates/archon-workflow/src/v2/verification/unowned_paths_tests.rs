//! Issue-81: a finding whose every cited path is declared by no task is
//! recorded for review instead of dispatched, and every other shape of
//! finding keeps today's behaviour exactly.
use std::collections::BTreeMap;

use serde_json::json;

use super::{UNOWNED_PATH_GAP_PREFIX, flag_unowned_path_gaps, gap_is_unowned_path, scope_by_item};
use crate::WorkflowV2Result;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::{WorkflowV2BranchOutcome, WorkflowV2FanoutItem, WorkflowV2Status};

const OWN: &str = "crates/engine/src/mine.rs";
const THEIRS: &str = "docs/plan.md";
const NOBODYS: &str = "crates/engine/src/support.rs";

/// A repository holding all three files, so "does this path exist" is a real
/// question and not a stub.
fn repository() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    for path in [OWN, THEIRS, NOBODYS] {
        let file = temp.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "// file\n").unwrap();
    }
    temp
}

fn task(id: &str, declared: &str) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        files_expected_to_change: vec![format!("`{declared}` — the deliverable")],
        ..Default::default()
    }
}

/// TASK-A owns its own file, TASK-B owns another; nobody declares `NOBODYS`.
fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "1".into(),
        source_roots: Vec::new(),
        tasks: vec![task("TASK-A", OWN), task("TASK-B", THEIRS)],
    }
}

fn by_item() -> BTreeMap<String, super::BranchScope> {
    let items = vec![WorkflowV2FanoutItem::read_only(
        "verify-1".to_string(),
        "coder".to_string(),
        crate::v2::WorkflowV2HostCall {
            id: "verification-wave-1-verify-1".into(),
            method: crate::v2::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        json!({"item": {"canonical_task_ids": ["TASK-A"], "target_files": [OWN]}}),
    )];
    scope_by_item(&items)
}

fn outcome(description: &str) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "verification failed".into(),
        ..WorkflowV2Result::default()
    };
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: "gap-live-lane".to_string(),
        description: description.to_string(),
        severity: Some("medium".to_string()),
    });
    WorkflowV2BranchOutcome {
        item_id: "verify-1".into(),
        role: "coder".into(),
        status: WorkflowV2Status::NeedsReview,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

fn flag(description: &str, temp: &std::path::Path) -> WorkflowV2BranchOutcome {
    let mut outcomes = vec![outcome(description)];
    flag_unowned_path_gaps(&mut outcomes, &by_item(), Some(&universe()), temp);
    outcomes.into_iter().next().unwrap()
}

fn gap(outcome: &WorkflowV2BranchOutcome) -> crate::WorkflowV2ResidualGap {
    outcome.result.as_ref().unwrap().residual_gaps[0].clone()
}

#[test]
fn a_gap_naming_only_paths_no_task_declares_is_flagged_and_asks_for_no_work() {
    let temp = repository();
    let outcome = flag(
        &format!("The live lane {NOBODYS}::ingest still hardcodes the default."),
        temp.path(),
    );
    // The verdict is the verifier's and is never touched here.
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        outcome.result.as_ref().unwrap().status,
        WorkflowV2Status::NeedsReview
    );
    let gap = gap(&outcome);
    assert_eq!(gap.id, format!("{UNOWNED_PATH_GAP_PREFIX}gap-live-lane"));
    assert_eq!(gap.severity.as_deref(), Some("review"));
    // The defect survives in full, and says why nobody was dispatched.
    assert!(
        gap.description.contains("still hardcodes the default"),
        "{gap:?}"
    );
    assert!(gap.description.contains(NOBODYS), "{gap:?}");
    assert!(gap.description.contains("no task"), "{gap:?}");
    let result = outcome.result.as_ref().unwrap();
    assert_eq!(result.data["unowned_finding_paths"], json!([NOBODYS]));
    assert_eq!(result.evidence.len(), 1);
    // And the dispatch predicate can see it for what it is.
    assert!(gap_is_unowned_path(&serde_json::to_value(&gap).unwrap()));
}

#[test]
fn a_gap_naming_a_path_in_the_branchs_own_scope_is_left_alone() {
    let temp = repository();
    let outcome = flag(
        &format!("{OWN} is missing the declared guard."),
        temp.path(),
    );
    let gap = gap(&outcome);
    assert_eq!(gap.id, "gap-live-lane");
    assert_eq!(gap.severity.as_deref(), Some("medium"));
    assert!(!gap_is_unowned_path(&serde_json::to_value(&gap).unwrap()));
    assert!(
        outcome
            .result
            .as_ref()
            .unwrap()
            .data
            .get("unowned_finding_paths")
            .is_none(),
        "{:?}",
        outcome.result
    );
}

#[test]
fn a_gap_naming_a_path_another_task_declares_is_left_alone() {
    let temp = repository();
    let outcome = flag(&format!("{THEIRS} contradicts the contract."), temp.path());
    let gap = gap(&outcome);
    assert_eq!(gap.id, "gap-live-lane");
    assert_eq!(gap.severity.as_deref(), Some("medium"));
}

#[test]
fn a_gap_naming_no_path_that_exists_is_left_alone() {
    let temp = repository();
    // Path-shaped, and nothing in the repository answers to it: a module
    // path, a rename or prose is not a citation.
    let outcome = flag(
        "crates/engine/src/ghost.rs and data_store::records are both wrong.",
        temp.path(),
    );
    let gap = gap(&outcome);
    assert_eq!(gap.id, "gap-live-lane");
    assert_eq!(gap.severity.as_deref(), Some("medium"));
}

#[test]
fn one_owned_path_alongside_an_unowned_one_leaves_the_whole_gap_alone() {
    let temp = repository();
    let outcome = flag(
        &format!("{NOBODYS} drifts from {OWN}, which is also wrong."),
        temp.path(),
    );
    assert_eq!(gap(&outcome).id, "gap-live-lane");
}

#[test]
fn without_a_task_universe_nothing_is_flagged() {
    let temp = repository();
    let mut outcomes = vec![outcome(&format!("{NOBODYS} is broken."))];
    flag_unowned_path_gaps(&mut outcomes, &by_item(), None, temp.path());
    assert_eq!(gap(&outcomes[0]).id, "gap-live-lane");
}

#[test]
fn an_outcome_with_no_captured_scope_is_left_alone() {
    let temp = repository();
    let mut outcomes = vec![outcome(&format!("{NOBODYS} is broken."))];
    flag_unowned_path_gaps(
        &mut outcomes,
        &BTreeMap::new(),
        Some(&universe()),
        temp.path(),
    );
    assert_eq!(gap(&outcomes[0]).id, "gap-live-lane");
}

/// A universe whose single task declares `path` exactly as written.
fn universe_declaring(entry: &str) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "1".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-B".into(),
            files_expected_to_change: vec![entry.to_string()],
            ..Default::default()
        }],
    }
}

fn flag_against(
    description: &str,
    universe: &WorkflowV2TaskUniverse,
    temp: &std::path::Path,
) -> WorkflowV2BranchOutcome {
    let mut outcomes = vec![outcome(description)];
    flag_unowned_path_gaps(&mut outcomes, &by_item(), Some(universe), temp);
    outcomes.into_iter().next().unwrap()
}

/// Issue-88, the false green. Live, a task declared its test file with an
/// ABSOLUTE path while the gap cited the repository-relative spelling. The
/// string comparison found no owner, the gap was excused as "no task declares
/// this", and a genuine in-scope failure — a test the acceptance criteria
/// named was never written — was excluded from remediation and offered to the
/// verifier as a reason to accept anyway.
#[test]
fn a_path_declared_absolutely_and_cited_relatively_is_owned_and_stays_blocking() {
    let temp = repository();
    let declared = format!(
        "`{}` — exists (492 lines)",
        temp.path().join(NOBODYS).display()
    );
    let outcome = flag_against(
        &format!("the AC-named test was never added to {NOBODYS} (still 9 tests)"),
        &universe_declaring(&declared),
        temp.path(),
    );
    let gap = gap(&outcome);
    assert_eq!(gap.id, "gap-live-lane", "a task DOES declare this file");
    assert_eq!(gap.severity.as_deref(), Some("medium"));
    assert!(!gap_is_unowned_path(&serde_json::to_value(&gap).unwrap()));
}

#[test]
fn a_path_declared_relatively_and_cited_relatively_is_owned_and_stays_blocking() {
    let temp = repository();
    let outcome = flag_against(
        &format!("{NOBODYS} is wrong"),
        &universe_declaring(&format!("`{NOBODYS}` — exists")),
        temp.path(),
    );
    assert_eq!(gap(&outcome).id, "gap-live-lane");
}

/// A declared DIRECTORY covers what is under it, in either spelling.
#[test]
fn a_declared_directory_above_the_cited_file_still_owns_it() {
    let temp = repository();
    for declared in [
        "`crates/engine/src`".to_string(),
        format!("`{}`", temp.path().join("crates/engine/src").display()),
    ] {
        let outcome = flag_against(
            &format!("{NOBODYS} is wrong"),
            &universe_declaring(&declared),
            temp.path(),
        );
        assert_eq!(gap(&outcome).id, "gap-live-lane", "{declared}");
    }
}

/// An absolute path in the prose is not extracted as a citation at all, so a
/// gap naming only one cites nothing and is left exactly as it was.
#[test]
fn a_gap_citing_only_an_absolute_path_cites_nothing_and_stays_blocking() {
    let temp = repository();
    let outcome = flag_against(
        &format!("{} is wrong", temp.path().join(NOBODYS).display()),
        &universe_declaring("`docs/unrelated.md`"),
        temp.path(),
    );
    assert_eq!(gap(&outcome).id, "gap-live-lane");
}

/// The only legitimate downgrade: nobody declares it, in either spelling.
#[test]
fn a_path_declared_by_no_task_in_either_form_is_the_one_case_that_downgrades() {
    let temp = repository();
    let outcome = flag_against(
        &format!("{NOBODYS} drifts from the contract"),
        &universe_declaring("`docs/unrelated.md`"),
        temp.path(),
    );
    assert_eq!(
        gap(&outcome).id,
        format!("{UNOWNED_PATH_GAP_PREFIX}gap-live-lane")
    );
}

/// Fail closed: one declared entry the host cannot read as a path refuses
/// every downgrade, because that entry might be the owner.
#[test]
fn an_uncanonicalisable_declaration_anywhere_refuses_every_downgrade() {
    let temp = repository();
    for unreadable in ["`crates/<dataset-id>/a.rs`", "`../outside.rs`", "``"] {
        let outcome = flag_against(
            &format!("{NOBODYS} drifts from the contract"),
            &universe_declaring(unreadable),
            temp.path(),
        );
        assert_eq!(gap(&outcome).id, "gap-live-lane", "{unreadable}");
    }
}

/// And a branch target the host cannot read refuses it too.
#[test]
fn an_uncanonicalisable_branch_target_refuses_the_downgrade() {
    let temp = repository();
    let mut outcomes = vec![outcome(&format!("{NOBODYS} drifts from the contract"))];
    let scope = BTreeMap::from([(
        "verify-1".to_string(),
        super::BranchScope {
            task_ids: vec!["TASK-A".into()],
            targets: vec!["crates/<dataset-id>/a.rs".into()],
        },
    )]);
    flag_unowned_path_gaps(
        &mut outcomes,
        &scope,
        Some(&universe_declaring("`docs/unrelated.md`")),
        temp.path(),
    );
    assert_eq!(gap(&outcomes[0]).id, "gap-live-lane");
}

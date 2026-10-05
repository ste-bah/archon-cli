//! Issue 322: the exit 75 contract states what exit 75 really promises. A
//! command that saves nothing (the unstaged CLI freeze) exits 75 too, so the
//! contract must not say that every such exit saved its work.

use super::reported_progress;
use crate::command::workflow_freeze_budget::FreezeIncomplete;

/// The module documentation of `workflow_host_command_operational.rs`, one
/// line, whitespace collapsed.
fn contract() -> String {
    include_str!("workflow_host_command_operational.rs")
        .lines()
        .map_while(|line| line.strip_prefix("//!"))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_exit_75_contract_names_a_command_that_saves_nothing() {
    let contract = contract();
    assert!(
        contract.contains("A command that saves nothing (the unstaged CLI freeze"),
        "{contract}"
    );
    assert!(
        contract.contains("a re-run of it starts over"),
        "{contract}"
    );
}

#[test]
fn the_exit_75_contract_never_promises_saved_work_unconditionally() {
    let contract = contract();
    assert!(
        !contract.contains("It persisted the work it finished"),
        "{contract}"
    );
    assert!(
        contract.contains("Exit 75 does NOT by itself mean that work was saved"),
        "{contract}"
    );
    let continues = contract
        .find("a re-run of the same call continues")
        .expect("the saving case is still stated");
    let saving = contract
        .find("A command that saves its work")
        .expect("the continue claim is scoped to a command that saves");
    assert!(saving < continues, "{contract}");
}

#[test]
fn the_unsaved_freeze_report_is_the_case_the_contract_names() {
    let report = FreezeIncomplete::unsaved("an unproven check").report();
    assert_eq!(reported_progress(report.as_bytes()), Some(0), "{report}");
    assert!(report.contains("saves no results"), "{report}");
    assert!(report.contains("starts the freeze again"), "{report}");
    assert!(
        contract().contains("reports progress 0 and says so in its reason"),
        "the contract names the progress the unsaved freeze reports"
    );
}

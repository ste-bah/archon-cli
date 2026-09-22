//! Issue-77: the fold itself — one entry per command string, attribution and
//! evidence unioned, the newer statement authoritative, order untouched.
use super::fold_landed_command;
use crate::{WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus};

fn record(
    command: &str,
    status: WorkflowV2CommandStatus,
    pre_existing: bool,
    output_summary: &str,
) -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: command.to_string(),
        status,
        exit_code: Some(if status == WorkflowV2CommandStatus::Failed {
            1
        } else {
            0
        }),
        output_summary: output_summary.to_string(),
        pre_existing,
    }
}

#[test]
fn a_new_command_string_is_appended_and_order_is_preserved() {
    let mut commands = vec![
        record("first", WorkflowV2CommandStatus::Succeeded, false, "ok"),
        record("second", WorkflowV2CommandStatus::Succeeded, false, "ok"),
    ];
    fold_landed_command(
        &mut commands,
        record("third", WorkflowV2CommandStatus::Failed, false, "boom"),
    );
    let order: Vec<&str> = commands
        .iter()
        .map(|command| command.command.as_str())
        .collect();
    assert_eq!(order, vec!["first", "second", "third"]);
}

#[test]
fn the_evidenced_attribution_survives_the_unattributed_copy() {
    let mut commands = vec![record(
        "gate",
        WorkflowV2CommandStatus::Failed,
        true,
        "owned by another task; absent at baseline too",
    )];
    fold_landed_command(
        &mut commands,
        record("gate", WorkflowV2CommandStatus::Failed, false, "exit 1"),
    );
    assert_eq!(commands.len(), 1, "{commands:#?}");
    assert!(commands[0].pre_existing, "{commands:#?}");
    assert_eq!(
        commands[0].output_summary,
        "owned by another task; absent at baseline too"
    );
}

#[test]
fn the_incoming_evidence_is_adopted_when_the_existing_copy_carries_none() {
    let mut commands = vec![record(
        "gate",
        WorkflowV2CommandStatus::Failed,
        false,
        "   ",
    )];
    fold_landed_command(
        &mut commands,
        record(
            "gate",
            WorkflowV2CommandStatus::Failed,
            true,
            "absent at baseline too",
        ),
    );
    assert_eq!(commands.len(), 1, "{commands:#?}");
    assert!(commands[0].pre_existing, "{commands:#?}");
    assert_eq!(commands[0].output_summary, "absent at baseline too");
}

#[test]
fn a_bare_flag_on_either_copy_does_not_launder_the_other_into_an_attribution() {
    let mut commands = vec![record("gate", WorkflowV2CommandStatus::Failed, true, "  ")];
    fold_landed_command(
        &mut commands,
        record(
            "gate",
            WorkflowV2CommandStatus::Failed,
            false,
            "assertion failed in the task's own test",
        ),
    );
    assert_eq!(commands.len(), 1, "{commands:#?}");
    assert!(!commands[0].pre_existing, "{commands:#?}");
}

#[test]
fn the_existing_statement_keeps_status_and_exit_code() {
    let mut commands = vec![record(
        "gate",
        WorkflowV2CommandStatus::Succeeded,
        false,
        "test result: ok. 3 passed",
    )];
    fold_landed_command(
        &mut commands,
        record("gate", WorkflowV2CommandStatus::Failed, false, "exit 1"),
    );
    assert_eq!(commands.len(), 1, "{commands:#?}");
    assert_eq!(commands[0].status, WorkflowV2CommandStatus::Succeeded);
    assert_eq!(commands[0].exit_code, Some(0));
}

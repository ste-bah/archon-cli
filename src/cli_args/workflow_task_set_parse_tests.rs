use clap::Parser;

use super::{Cli, Commands, WorkflowAction};

#[test]
fn workflow_task_set_commands_and_task_file_lint_parse() {
    let lint = Cli::try_parse_from([
        "archon",
        "workflow",
        "lint",
        "--task-file",
        "tasks/TASK-X-010.md",
    ])
    .unwrap();
    match lint.command.unwrap() {
        Commands::Workflow {
            action:
                WorkflowAction::Lint {
                    task_file: Some(path),
                    tasks: None,
                    spec_file: None,
                    graph: None,
                },
        } => assert_eq!(path, std::path::PathBuf::from("tasks/TASK-X-010.md")),
        other => panic!("unexpected action: {other:?}"),
    }

    let freeze = Cli::try_parse_from([
        "archon",
        "workflow",
        "freeze-skeleton",
        "--tasks",
        "tasks/PRD-X",
        "--prd",
        "prds/PRD-X.md",
    ])
    .unwrap();
    match freeze.command.unwrap() {
        Commands::Workflow {
            action: WorkflowAction::FreezeSkeleton { tasks, prd },
        } => {
            assert_eq!(tasks, std::path::PathBuf::from("tasks/PRD-X"));
            assert_eq!(prd, std::path::PathBuf::from("prds/PRD-X.md"));
        }
        other => panic!("unexpected action: {other:?}"),
    }
    assert!(
        Cli::try_parse_from([
            "archon",
            "workflow",
            "freeze-skeleton",
            "--tasks",
            "tasks/PRD-X",
        ])
        .is_err(),
        "freeze-skeleton requires the explicit PRD identity"
    );
}

#[test]
fn gate_command_help_describes_mode_dependent_exit_status() {
    let lint_help = Cli::try_parse_from(["archon", "workflow", "lint", "--help"])
        .unwrap_err()
        .to_string();
    assert!(lint_help.contains("gate_mode"), "{lint_help}");
    assert!(!lint_help.contains("No lint here can fail"), "{lint_help}");

    let trace_help = Cli::try_parse_from(["archon", "requirements", "trace", "--help"])
        .unwrap_err()
        .to_string();
    assert!(trace_help.contains("gate_mode"), "{trace_help}");
    assert!(
        !trace_help.contains("exit status is success either way"),
        "{trace_help}"
    );
}

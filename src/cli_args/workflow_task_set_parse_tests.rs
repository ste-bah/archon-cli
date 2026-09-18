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
                    candidate_stdin: false,
                    staging_root: None,
                    gate_envelope: None,
                    call_id: None,
                    fidelity: false,
                    waive_obligation,
                    waive_reason: None,
                },
        } => {
            assert_eq!(path, std::path::PathBuf::from("tasks/TASK-X-010.md"));
            assert!(waive_obligation.is_empty());
        }
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
            action: WorkflowAction::FreezeSkeleton { tasks, prd, .. },
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
fn workflow_decompose_parses_prd_tasks_and_yes() {
    let cli = Cli::try_parse_from([
        "archon",
        "workflow",
        "decompose",
        "--prd",
        "prds/PRD-X.md",
        "--tasks",
        "tasks/PRD-X",
        "--yes",
    ])
    .unwrap();

    match cli.command.unwrap() {
        Commands::Workflow {
            action: WorkflowAction::Decompose { prd, tasks, yes },
        } => {
            assert_eq!(prd, std::path::PathBuf::from("prds/PRD-X.md"));
            assert_eq!(tasks, std::path::PathBuf::from("tasks/PRD-X"));
            assert!(yes);
        }
        other => panic!("unexpected action: {other:?}"),
    }
}

#[test]
fn workflow_decompose_requires_prd_and_tasks() {
    assert!(
        Cli::try_parse_from([
            "archon",
            "workflow",
            "decompose",
            "--tasks",
            "tasks/PRD-X",
            "--yes",
        ])
        .is_err(),
        "decompose requires --prd"
    );
    assert!(
        Cli::try_parse_from([
            "archon",
            "workflow",
            "decompose",
            "--prd",
            "prds/PRD-X.md",
            "--yes",
        ])
        .is_err(),
        "decompose requires --tasks"
    );
}

#[test]
fn workflow_decompose_does_not_add_continue_alias() {
    let err = Cli::try_parse_from([
        "archon",
        "workflow",
        "continue",
        "--prd",
        "prds/PRD-X.md",
        "--tasks",
        "tasks/PRD-X",
        "--yes",
    ])
    .unwrap_err()
    .to_string();

    assert!(err.contains("unexpected argument '--prd'"), "{err}");
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

#[test]
fn workflow_decomposition_identity_parses_without_live_flags() {
    let cli = Cli::try_parse_from(["archon", "workflow", "decomposition-identity"]).unwrap();
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Workflow {
            action: WorkflowAction::DecompositionIdentity
        }
    ));
}

#[test]
fn reclaim_task_root_requires_named_run_and_explicit_confirmation() {
    assert!(Cli::try_parse_from(["archon", "workflow", "reclaim-task-root"]).is_err());
    let cli = Cli::try_parse_from([
        "archon",
        "workflow",
        "reclaim-task-root",
        "wf-dead-owner",
        "--yes",
    ])
    .expect("operator must have an explicit evidence-preserving reclaim command");
    match cli.command.unwrap() {
        Commands::Workflow {
            action: WorkflowAction::ReclaimTaskRoot { run_id, yes },
        } => {
            assert_eq!(run_id, "wf-dead-owner");
            assert!(yes);
        }
        other => panic!("unexpected action: {other:?}"),
    }
}

#[test]
fn verify_frozen_chain_parses_the_staged_child_form_only() {
    let cli = Cli::try_parse_from([
        "archon",
        "workflow",
        "verify-frozen-chain",
        "--stage",
        "skeleton",
        "--tasks",
        "tasks/PRD-X",
        "--prd",
        "prds/PRD-X.md",
        "--gate-envelope",
        "staging/gate-envelope.json",
        "--call-id",
        "call-1",
    ])
    .unwrap();
    match cli.command.unwrap() {
        Commands::Workflow {
            action:
                WorkflowAction::VerifyFrozenChain {
                    stage,
                    tasks,
                    prd,
                    gate_envelope,
                    call_id,
                },
        } => {
            assert_eq!(stage, "skeleton");
            assert_eq!(tasks, std::path::PathBuf::from("tasks/PRD-X"));
            assert_eq!(prd, std::path::PathBuf::from("prds/PRD-X.md"));
            assert_eq!(
                gate_envelope,
                Some(std::path::PathBuf::from("staging/gate-envelope.json"))
            );
            assert_eq!(call_id.as_deref(), Some("call-1"));
        }
        other => panic!("unexpected action {other:?}"),
    }
    assert!(
        Cli::try_parse_from([
            "archon",
            "workflow",
            "verify-frozen-chain",
            "--tasks",
            "t",
            "--prd",
            "p"
        ])
        .is_err(),
        "--stage is required"
    );
}

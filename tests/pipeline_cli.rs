use archon_cli_workspace::cli_args::{Cli, Commands, PipelineAction};
use clap::Parser;

#[test]
fn test_pipeline_code_parses() {
    let cli =
        Cli::try_parse_from(["archon", "pipeline", "code", "implement hello world"]).unwrap();
    match cli.command {
        Some(Commands::Pipeline { action: PipelineAction::Code { task, dry_run } }) => {
            assert_eq!(task, "implement hello world");
            assert!(!dry_run);
        }
        other => panic!("expected Pipeline Code, got {:?}", other),
    }
}

#[test]
fn test_pipeline_code_dry_run_parses() {
    let cli =
        Cli::try_parse_from(["archon", "pipeline", "code", "--dry-run", "test task"]).unwrap();
    match cli.command {
        Some(Commands::Pipeline { action: PipelineAction::Code { dry_run, .. } }) => {
            assert!(dry_run);
        }
        other => panic!("expected Pipeline Code with dry_run, got {:?}", other),
    }
}

#[test]
fn test_pipeline_research_parses() {
    let cli =
        Cli::try_parse_from(["archon", "pipeline", "research", "quantum computing"]).unwrap();
    match cli.command {
        Some(Commands::Pipeline { action: PipelineAction::Research { topic, dry_run } }) => {
            assert_eq!(topic, "quantum computing");
            assert!(!dry_run);
        }
        other => panic!("expected Pipeline Research, got {:?}", other),
    }
}

#[test]
fn test_pipeline_status_parses() {
    let cli = Cli::try_parse_from(["archon", "pipeline", "status", "abc-123"]).unwrap();
    match cli.command {
        Some(Commands::Pipeline { action: PipelineAction::Status { session_id } }) => {
            assert_eq!(session_id, "abc-123");
        }
        other => panic!("expected Pipeline Status, got {:?}", other),
    }
}

#[test]
fn test_pipeline_resume_parses() {
    let cli = Cli::try_parse_from(["archon", "pipeline", "resume", "abc-123"]).unwrap();
    match cli.command {
        Some(Commands::Pipeline { action: PipelineAction::Resume { session_id } }) => {
            assert_eq!(session_id, "abc-123");
        }
        other => panic!("expected Pipeline Resume, got {:?}", other),
    }
}

#[test]
fn test_pipeline_list_parses() {
    let cli = Cli::try_parse_from(["archon", "pipeline", "list"]).unwrap();
    match cli.command {
        Some(Commands::Pipeline { action: PipelineAction::List }) => {}
        other => panic!("expected Pipeline List, got {:?}", other),
    }
}

#[test]
fn test_pipeline_abort_parses() {
    let cli = Cli::try_parse_from(["archon", "pipeline", "abort", "abc-123"]).unwrap();
    match cli.command {
        Some(Commands::Pipeline { action: PipelineAction::Abort { session_id } }) => {
            assert_eq!(session_id, "abc-123");
        }
        other => panic!("expected Pipeline Abort, got {:?}", other),
    }
}

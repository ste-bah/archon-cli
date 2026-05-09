use super::{Cli, Commands, SandboxAction};
use clap::Parser;

#[test]
fn sandbox_sessions_parses_filters_and_json() {
    let cli = Cli::try_parse_from([
        "archon",
        "sandbox",
        "sessions",
        "--status",
        "configured",
        "--agent",
        "reviewer",
        "--limit",
        "5",
        "--json",
    ])
    .expect("sandbox sessions must parse");

    match cli.command {
        Some(Commands::Sandbox {
            action:
                Some(SandboxAction::Sessions {
                    status,
                    agent,
                    limit,
                    json,
                }),
        }) => {
            assert_eq!(status.as_deref(), Some("configured"));
            assert_eq!(agent.as_deref(), Some("reviewer"));
            assert_eq!(limit, 5);
            assert!(json);
        }
        other => panic!("expected sandbox sessions, got {other:?}"),
    }
}

#[test]
fn sandbox_explain_parses_tool_and_command() {
    let cli = Cli::try_parse_from([
        "archon",
        "sandbox",
        "explain",
        "--backend",
        "openshell",
        "--tool",
        "Bash",
        "--command",
        "cargo test -p archon-core",
    ])
    .expect("sandbox explain must parse");

    match cli.command {
        Some(Commands::Sandbox {
            action:
                Some(SandboxAction::Explain {
                    backend,
                    tool,
                    command,
                }),
        }) => {
            assert_eq!(backend.as_deref(), Some("openshell"));
            assert_eq!(tool.as_deref(), Some("Bash"));
            assert_eq!(command.as_deref(), Some("cargo test -p archon-core"));
        }
        other => panic!("expected sandbox explain, got {other:?}"),
    }
}
